use std::{io, path::Path};
pub trait LocalStream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> LocalStream for T {}
pub type Stream = Box<dyn LocalStream>;

#[cfg(windows)]
mod native {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    fn sid() -> io::Result<String> {
        unsafe {
            let mut token = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let result = (|| {
                let mut size = 0;
                GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
                if size == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut data = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
                if GetTokenInformation(token, TokenUser, data.as_mut_ptr().cast(), size, &mut size)
                    == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let user = &*data.as_ptr().cast::<TOKEN_USER>();
                let mut text = std::ptr::null_mut();
                if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut len = 0;
                while *text.add(len) != 0 {
                    len += 1;
                }
                let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
                LocalFree(text.cast());
                Ok(result)
            })();
            CloseHandle(token);
            result
        }
    }
    fn name(root: &Path) -> io::Result<String> {
        Ok(format!(
            r"\\.\pipe\nuphus-workbench-{}-{}",
            sid()?,
            super::super::profile_key(root)?
        ))
    }
    fn pipe(name: &str, first: bool) -> io::Result<NamedPipeServer> {
        let descriptor: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", sid()?)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            let mut security = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor.as_ptr(),
                1,
                &mut security,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: security,
                bInheritHandle: 0,
            };
            let result = ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    name,
                    (&attributes as *const SECURITY_ATTRIBUTES)
                        .cast_mut()
                        .cast(),
                );
            LocalFree(security);
            result
        }
    }
    pub struct Listener {
        name: String,
        pending: NamedPipeServer,
    }
    impl Listener {
        pub fn bind(root: &Path) -> io::Result<Self> {
            let name = name(root)?;
            Ok(Self {
                pending: pipe(&name, true)?,
                name,
            })
        }
        pub async fn accept(&mut self) -> io::Result<Stream> {
            self.pending.connect().await?;
            let next = pipe(&self.name, false)?;
            Ok(Box::new(std::mem::replace(&mut self.pending, next)))
        }
    }
    pub async fn connect(root: &Path) -> io::Result<Stream> {
        Ok(Box::new(ClientOptions::new().open(name(root)?)?))
    }
    pub fn unavailable(error: &io::Error) -> bool {
        matches!(error.raw_os_error(), Some(2 | 3 | 231)) // absent / busy pipe
    }
}

#[cfg(unix)]
mod native {
    use super::*;
    use std::{
        os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        path::PathBuf,
    };
    fn path(root: &Path) -> io::Result<PathBuf> {
        let uid = unsafe { libc::geteuid() };
        // Keep below sockaddr_un's path limit even with long macOS profile paths.
        let directory = PathBuf::from("/tmp").join(format!("nuphus-workbench-{uid}"));
        match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = std::fs::symlink_metadata(&directory)?;
        if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "IPC directory must be owned by this user with mode 0700",
            ));
        }
        Ok(directory.join(format!("{}.sock", super::super::profile_key(root)?)))
    }
    pub struct Listener {
        socket: tokio::net::UnixListener,
        path: PathBuf,
    }
    impl Listener {
        pub fn bind(root: &Path) -> io::Result<Self> {
            let path = path(root)?;
            if let Ok(meta) = std::fs::symlink_metadata(&path) {
                if !meta.file_type().is_socket() || meta.uid() != unsafe { libc::geteuid() } {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Unexpected IPC path",
                    ));
                }
                if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "Workbench IPC is already running",
                    ));
                }
                // Native host owns host.lock before binding; only its stale socket is removed.
                std::fs::remove_file(&path)?;
            }
            let socket = tokio::net::UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { socket, path })
        }
        pub async fn accept(&mut self) -> io::Result<Stream> {
            Ok(Box::new(self.socket.accept().await?.0))
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    pub async fn connect(root: &Path) -> io::Result<Stream> {
        Ok(Box::new(
            tokio::net::UnixStream::connect(path(root)?).await?,
        ))
    }
    pub fn unavailable(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
        )
    }
}
pub use native::*;
