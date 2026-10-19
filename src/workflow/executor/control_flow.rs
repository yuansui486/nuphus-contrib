//! 控制流信号载体（B2/B3）
//!
//! break/continue 不能用普通返回字符串表达：用户脚本/工具输出可能恰好是
//! "break"/"continue"（字符串碰撞），且普通 Ok(String) 无法穿透 Seq/If 容器。
//! 借助错误通道：`Err(ControlFlow)` 沿 `?` 向上传播，由最近一层 loop 捕获消费；
//! dispatch 末尾统一还原为「正常完成」的记录与事件（控制流不是失败）。
//!
//! 设计对齐 `DesktopActionError` 的「结构化信号穿越既有通道」模式——
//! 这里用错误通道而非输出字符串，因为错误通道是执行器间唯一的强制传播路径。

/// 循环控制流信号
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlow {
    Break,
    Continue,
}

impl ControlFlow {
    /// 错误通道内的信号编码前缀。专有前缀杜绝与用户工具输出碰撞。
    pub const BREAK_MESSAGE: &'static str = "\u{0}nuphus:control-flow:break";
    pub const CONTINUE_MESSAGE: &'static str = "\u{0}nuphus:control-flow:continue";

    pub fn message(self) -> &'static str {
        match self {
            ControlFlow::Break => Self::BREAK_MESSAGE,
            ControlFlow::Continue => Self::CONTINUE_MESSAGE,
        }
    }

    /// 从错误消息还原控制流信号；非控制流消息返回 None。
    pub fn decode(message: &str) -> Option<Self> {
        match message {
            Self::BREAK_MESSAGE => Some(ControlFlow::Break),
            Self::CONTINUE_MESSAGE => Some(ControlFlow::Continue),
            _ => None,
        }
    }
}

/// 由信号构造的 NuphusError——控制流借错误通道向上传播。
pub(crate) fn control_flow_error(signal: ControlFlow) -> crate::NuphusError {
    crate::NuphusError::agent(signal.message())
}

/// 判断一个错误是否是控制流信号。
pub(crate) fn is_control_flow_error(error: &crate::NuphusError) -> Option<ControlFlow> {
    match error {
        crate::NuphusError::Agent(crate::AgentError::Other(message)) => {
            ControlFlow::decode(message)
        }
        _ => None,
    }
}
