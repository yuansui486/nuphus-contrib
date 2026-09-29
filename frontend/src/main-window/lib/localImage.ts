/**
 * localImage — 本地图片「选图 → 入库」的单一实现点。
 *
 * 消费方：`AppearancePanel`（皮肤背景）与 `SoulPage` 头像区（用户 / Nuphus 头像）。
 * 两侧本来就是同一套本地架构（选一张图 → 复制进应用数据目录 → 只存路径），
 * 早先各写一份逐字相同的 `pickAndImportImage`；抽到这里只为消除这份重复，
 * 语义与实现一字未改。
 *
 * 为什么是「入库后只存路径」而不是「读成 base64 塞进 localStorage」：
 *   图片本来就落在本地磁盘上。把文件复制进 `nuphus_data_dir()/images/` 后只记路径，
 *   渲染侧用 `toAssetUrl(path)` 走 asset:// 读出来。由此同时消掉三件事：
 *   localStorage 5MB 配额被大图占满导致后续写入（含主题）静默失败、
 *   数 MB base64 刷新后常驻内存、以及「用户移动/删除原文件 → 背景失效」。
 *
 * 为什么走 `plugin-dialog` 的 open() 而非 `<input type=file>`：
 *   后者只给 File 对象，拿不到可靠的绝对路径（本项目前端历来只用 FileReader
 *   读 dataURL）。
 */

/** 选一张本地图片并**入库**（复制进应用数据目录），返回本地方径；取消返回 null */
export async function pickAndImportImage(): Promise<string | null> {
  const { open } = await import('@tauri-apps/plugin-dialog')
  const selected = await open({
    multiple: false,
    filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'] }],
  })
  if (!selected || typeof selected !== 'string') return null
  const { invoke } = await import('@tauri-apps/api/core')
  return await invoke<string>('save_user_image', { sourcePath: selected })
}
