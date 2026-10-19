// DesktopToolbar.tsx — 桌面工具浮窗条 v6
// 右侧固定按钮列（dock 常驻 + hover 从上到下逐次显形），**不可拖拽**、无快捷键、无置顶
// Screenshot/region/OCR overlay tools: start_overlay_mask returns immediately, poll take_capture_result for results
// Completely solve Tauri event loss / oneshot blocking null issue

import { useState, useRef, useEffect, useCallback } from 'react'
import { invoke as bridgeInvoke, listen } from '../../core/bridge'
import { showAppFeedback } from '../../ui/islandChannel'
import { useLanguage } from '../../locales'

async function tauriInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    return (await invoke(cmd, args)) as T
  } catch (e: any) {
    const parts: string[] = []
    if (e?.message) parts.push(e.message)
    if (typeof e === 'string') parts.push(e)
    try {
      parts.push(JSON.stringify(e))
    } catch {}
    const msg = parts.filter(Boolean).join(' | ') || '未知错误'
    throw new Error(`Tauri invoke ${cmd} failed: ${msg}`)
  }
}

import {
  IconCamera,
  IconCrop,
  IconCrosshair,
  IconType,
  IconX,
  IconCopy,
  IconCheck,
  IconAppWindow,
  IconWrench,
} from '../../ui/Icons'

import { Pipette as IconDropper } from 'lucide-react'

import { OcrDictionary } from './OcrDictionary'
import { Button, IconButton } from '../../ui/Button'

type ToolMode = null | 'screenshot' | 'picker' | 'mouse_pos' | 'ocr' | 'color_picker'

type ResultType = 'text' | 'ocr' | 'info'

interface ToolResult {
  type: ResultType
  content: string
  title: string
}

interface ColorSlot {
  hex: string
  rgb: number[]
  filled: boolean
}

interface ToolBtn {
  mode: ToolMode
  icon: React.ElementType
  /** i18n key（locales workflowEditor.ts 的 toolbar.* 段） */
  labelKey: string
  /** 快捷键（显示在 hover 浮层第二行） */
  hint?: string
  /** 补充说明（显示在 hover 浮层第二行，hint 缺省时） */
  detail?: string
}

const TOOLS: ToolBtn[] = [
  {
    mode: 'screenshot',
    icon: IconCamera,
    labelKey: 'toolbar.screenshot',
    hint: 'Ctrl+1',
    detail: '截图保存',
  },
  {
    mode: 'picker',
    icon: IconCrop,
    labelKey: 'toolbar.picker',
    hint: 'Ctrl+2',
    detail: '框选区域',
  },
  {
    mode: 'color_picker',
    icon: IconDropper,
    labelKey: 'toolbar.colorPicker',
    hint: 'Ctrl+3',
    detail: '屏幕取色',
  },
  { mode: 'mouse_pos', icon: IconCrosshair, labelKey: 'toolbar.mousePos', detail: '实时坐标' },
  { mode: 'ocr', icon: IconType, labelKey: 'toolbar.ocr', detail: '文字识别' },
]

/**
 * 渐显后的常驻时长（ms）：3 秒。
 * 常驻期间鼠标移开也不渐隐；再次进入会续期。
 */
const HOVER_HOLD_MS = 3000

export function DesktopToolbar({ onOpenTools }: { onOpenTools?: () => void } = {}) {
  const { t } = useLanguage()
  // Sub-panel state
  const [activeTool, setActiveTool] = useState<ToolMode>(null)
  const [result, setResult] = useState<ToolResult | null>(null)
  const [loading, setLoading] = useState(false)
  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Mouse position live listener
  const [cursorPos, setCursorPos] = useState({ x: 0, y: 0 })
  const cursorInterval = useRef<ReturnType<typeof setInterval> | null>(null)

  const [copied, setCopied] = useState(false)

  // ── Color picker 9-slot state ──
  const initSlots = () =>
    Array.from({ length: 9 }, (): ColorSlot => ({ hex: '#000000', rgb: [0, 0, 0], filled: false }))
  const [colorSlots, setColorSlots] = useState<ColorSlot[]>(initSlots)
  const [pickingSlotIndex, setPickingSlotIndex] = useState<number | null>(null)
  const pickingSlotIndexRef = useRef<number | null>(null)
  const [colorCopiedIndex, setColorCopiedIndex] = useState<number | null>(null)

  // Dictionary OCR settings panel
  const [showOcrDict, setShowOcrDict] = useState(false)

  // ── 显隐：边缘防抖 + 渐显后常驻 3s ──
  // 语义（大王 2026-10-17 定）：
  //   ① 鼠标进入 dock → 需连续停留 HOVER_ENTER_DELAY_MS 才渐显（扫过右缘的误触不触发）
  //   ② 渐显后无论鼠标在不在，都保持 HOLD_MS 常驻；期间再次进入/停留会续期
  //   ③ 常驻结束后鼠标已不在 dock → 渐隐；仍在 → 继续常驻
  //   ④ 工具面板打开、取色/截图遮罩流程中、坐标工具激活 → 强制常驻（不因移开鼠标消失）
  // 用定时器而不是 CSS :hover：CSS 无法表达「进入防抖」与「延时保持」，且
  // JS 侧必须知道当前是否显形才能正确续期。
  const [dockShown, setDockShown] = useState(false)
  const dockRef = useRef<HTMLDivElement | null>(null)
  /** 与 dockShown 同步的镜像：pin 解除的 effect 需要读「当前是否显形」，
   *  写进依赖数组会让每次显隐都重跑该 effect，用 ref 读最新值更稳。 */
  const dockShownRef = useRef(false)
  useEffect(() => {
    dockShownRef.current = dockShown
  }, [dockShown])
  const enterTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const holdTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** 鼠标当前是否在 dock 上。**不用 `el.matches(':hover')`**：jsdom 不支持该伪类
   *  （恒为 false），且在定时器回调里读 DOM 也不如自己记账可靠。 */
  const hoveringRef = useRef(false)

  /** 续期常驻窗口：从此刻起再保持 HOVER_HOLD_MS。
   *  到点时鼠标仍在 dock 上 → 递归续期（用 ref 持有自身，避免 useCallback 自引用）。 */
  const renewHoldRef = useRef<() => void>(() => {})
  const renewHold = useCallback(() => {
    if (holdTimerRef.current) clearTimeout(holdTimerRef.current)
    holdTimerRef.current = setTimeout(() => {
      holdTimerRef.current = null
      if (hoveringRef.current) {
        renewHoldRef.current()
        return
      }
      setDockShown(false)
    }, HOVER_HOLD_MS)
  }, [])
  renewHoldRef.current = renewHold

  /** 强制常驻（面板/遮罩流程期间）：显形且不安排任何收起 */
  const pinShown = useCallback(() => {
    if (enterTimerRef.current) {
      clearTimeout(enterTimerRef.current)
      enterTimerRef.current = null
    }
    if (holdTimerRef.current) {
      clearTimeout(holdTimerRef.current)
      holdTimerRef.current = null
    }
    setDockShown(true)
  }, [])

  /** 鼠标进入：立即显形（不做防抖延迟 —— 悬停必须跟手） */
  const onDockEnter = useCallback(() => {
    hoveringRef.current = true
    if (enterTimerRef.current) {
      clearTimeout(enterTimerRef.current)
      enterTimerRef.current = null
    }
    setDockShown(true)
    renewHold()
  }, [renewHold])

  /** 鼠标离开：显形后靠常驻计时器自然渐隐（3s 内再进入会续期） */
  const onDockLeave = useCallback(() => {
    hoveringRef.current = false
    if (enterTimerRef.current) {
      clearTimeout(enterTimerRef.current)
      enterTimerRef.current = null
    }
  }, [])

  // 卸载清理：两个定时器都必须撤，否则卸载后仍会 setState
  useEffect(() => {
    return () => {
      if (enterTimerRef.current) clearTimeout(enterTimerRef.current)
      if (holdTimerRef.current) clearTimeout(holdTimerRef.current)
    }
  }, [])

  // 「面板/流程进行中」→ 强制常驻。这类状态下列表必须可见（否则用户无法收尾）。
  const dockPinned = activeTool !== null || showOcrDict || loading || pickingSlotIndex !== null
  useEffect(() => {
    if (dockPinned) {
      pinShown()
      return
    }
    // 钉住解除（面板关闭 / 工具退出 / 轮询结束）：pinShown 已把常驻计时器清掉，
    // 若此刻不重新起一次计时，dock 会停在显形态永不收起 —— 这是「显示之后不
    // 消失」的唯一成因。鼠标仍在 dock 上时，renewHold 到点会自行续期。
    if (dockShownRef.current) renewHold()
  }, [dockPinned, pinShown, renewHold])

  // ── Mouse position polling ──
  // ⚠️ 失败必须可见：`desktop_mouse_position` 走资源门，Agent 轮次 / 工作流运行
  // 期间会被以 `automation_busy` 拒绝。**旧实现静默 catch 掉**，读数值就永远停在
  // 初值 (0, 0) —— 用户看到「点击无效」，实际是被拒且无人告知。
  // 现在：连续失败即停轮询并把原因显示在读数位上（点开才知道为什么没数）。
  const [posError, setPosError] = useState<string | null>(null)
  useEffect(() => {
    if (activeTool === 'mouse_pos') {
      let failures = 0
      setPosError(null)
      const poll = async () => {
        try {
          const r = await bridgeInvoke<{ x: number; y: number }>('desktop_mouse_position')
          if (r) {
            setCursorPos(r)
            setPosError(null)
            failures = 0
          }
        } catch (e) {
          failures += 1
          // 单次抖动不打断（60ms 轮询，偶发一次失败不代表资源被占）
          if (failures === 1) return
          if (cursorInterval.current) clearInterval(cursorInterval.current)
          cursorInterval.current = null
          setPosError(String(e))
        }
      }
      poll()
      cursorInterval.current = setInterval(poll, 60)
    } else {
      if (cursorInterval.current) clearInterval(cursorInterval.current)
      cursorInterval.current = null
      setPosError(null)
    }
    return () => {
      if (cursorInterval.current) clearInterval(cursorInterval.current)
    }
  }, [activeTool])

  // ── Stop polling ──
  const stopPolling = useCallback(() => {
    if (pollRef.current) {
      clearInterval(pollRef.current)
      pollRef.current = null
    }
  }, [])

  // ── Poll take_capture_result ──
  const startPolling = useCallback(
    (mode: ToolMode) => {
      stopPolling()
      pollRef.current = setInterval(async () => {
        try {
          const raw = await tauriInvoke<any>('take_capture_result')
          if (raw === null || raw === undefined) {
            // No result yet, continue polling
            return
          }
          // Got result! Stop polling
          stopPolling()
          setLoading(false)

          // Process result —— 取消 / 保存到桌面走 island 轻提示，不弹窗（用户少一次
          // 点击关闭）；弹窗只留给「成功且可能要复制」的场景（见下方 picker 分支）。
          if (raw.cancelled) {
            if (pickingSlotIndexRef.current !== null) {
              // Color picker cancelled: return to panel
              setPickingSlotIndex(null)
              pickingSlotIndexRef.current = null
              setLoading(false)
              showAppFeedback('已取消取色', 'info')
              return
            }
            setActiveTool(null)
            showAppFeedback('已取消', 'info')
            return
          }

          // 保存到桌面（不注入对话）：截图复制一份到桌面方便用户其它用途。
          // 不派发 capture-result 事件（「确认」= 进对话的出口）。
          if (raw.saved_only) {
            setActiveTool(null)
            showAppFeedback(`截图已保存到桌面：${raw.path || ''}`, 'success')
            return
          }

          const path = raw.path || ''
          const region = raw.region || {}

          switch (mode) {
            case 'color_picker': {
              const slotIdx = pickingSlotIndexRef.current
              if (slotIdx === null) break
              const color = (raw.color_rgb as number[]) || [0, 0, 0]
              const hex = raw.hex || '#000000'
              setColorSlots(prev => {
                const next = [...prev]
                next[slotIdx] = { hex, rgb: color, filled: true }
                return next
              })
              setPickingSlotIndex(null)
              pickingSlotIndexRef.current = null
              setLoading(false)
              return // Don't show result popup, stay in panel
            }
            case 'picker':
              setActiveTool(null)
              setResult({
                type: 'text',
                title: '选区坐标',
                content: `选区: (${region.x}, ${region.y})  ${region.width} × ${region.height}`,
              })
              break
            default:
              // Screenshot: dispatch to ReferenceBar via CustomEvent (no popup, no tool_call wrapping)
              // The user decides whether to OCR, find_image, or just analyze — we only pass the path.
              setActiveTool(null)
              window.dispatchEvent(
                new CustomEvent('nuphus:capture-result', {
                  detail: { path, region, base64: raw.base64 || null },
                }),
              )
              break
          }
        } catch (e: any) {
          // Only stop on take_capture_result command error
          console.error('[DesktopToolbar] poll error:', e)
          stopPolling()
          setLoading(false)
          if (pickingSlotIndexRef.current === null) {
            setActiveTool(null)
            showAppFeedback(`轮询错误：${String(e)}`, 'error')
          } else {
            // Color picker polling error: return to panel
            setPickingSlotIndex(null)
            pickingSlotIndexRef.current = null
          }
        }
      }, 500) // 500ms polling interval
    },
    [stopPolling],
  )

  // ── Cleanup ──
  useEffect(() => {
    return () => {
      stopPolling()
    }
  }, [stopPolling])

  // ── Global Esc exits active tool (mouse pos, input panel, etc.) ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && activeTool !== null) {
        setActiveTool(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [activeTool])

  // ── Tool click ──
  const handleToolClick = async (mode: ToolMode) => {
    setResult(null)
    setCopied(false)

    if (mode === 'mouse_pos') {
      setActiveTool(prev => (prev === 'mouse_pos' ? null : 'mouse_pos'))
      return
    }

    if (mode === 'ocr') {
      setShowOcrDict(true)
      return
    }

    if (mode === 'color_picker') {
      if (activeTool === 'color_picker' && pickingSlotIndex === null) {
        setActiveTool(null)
      } else {
        setActiveTool('color_picker')
        setPickingSlotIndex(null)
        pickingSlotIndexRef.current = null
      }
      return
    }

    setActiveTool(mode)
    setLoading(true)
    try {
      // start_overlay_mask: pre-capture fullscreen → hide main window → create overlay → return immediately
      // Result obtained via polling take_capture_result
      // Start polling, wait for overlay_capture_done/cancel
      await tauriInvoke<any>('start_overlay_mask', { mode })
      startPolling(mode)
    } catch (e: any) {
      showAppFeedback(`操作失败：${String(e)}`, 'error')
      setLoading(false)
      setActiveTool(null)
    }
  }

  // ── Color slot click ──
  const handleColorSlotPick = async (index: number) => {
    setPickingSlotIndex(index)
    pickingSlotIndexRef.current = index
    setLoading(true)
    try {
      await tauriInvoke<any>('start_overlay_mask', { mode: 'color_picker' })
      startPolling('color_picker')
    } catch (e: any) {
      setPickingSlotIndex(null)
      pickingSlotIndexRef.current = null
      setLoading(false)
    }
  }

  // ── 全局工具热键（Ctrl+1/2/3）：后端解出工具名后派发事件，这里执行 ──
  // 点击与热键走同一条执行链（handleToolClick），避免两套逻辑漂移。
  // 用 ref 持有最新 handler：事件订阅只建一次，不因 activeTool 变化反复重订阅。
  const hotkeyHandlerRef = useRef<(tool: string) => void>(() => {})
  hotkeyHandlerRef.current = (tool: string) => {
    if (tool === 'screenshot' || tool === 'picker' || tool === 'color_picker') {
      void handleToolClick(tool)
      return
    }
    console.warn('[DesktopToolbar] unknown tool hotkey:', tool)
  }
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    listen<{ tool?: string }>('desktop-tool-hotkey', payload => {
      if (disposed) return
      const tool = payload?.tool
      if (typeof tool === 'string') hotkeyHandlerRef.current(tool)
    })
      .then(fn => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(e => {
        console.warn('[DesktopToolbar] hotkey listener failed:', e)
      })
    return () => {
      disposed = true
      if (unlisten) unlisten()
    }
  }, [])

  // ── Copy result ──
  const handleCopyResult = async () => {
    if (!result) return
    try {
      await navigator.clipboard.writeText(result.content)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      try {
        await bridgeInvoke('desktop_clipboard_write', {
          text: result.content,
        })
        setCopied(true)
        setTimeout(() => setCopied(false), 2000)
      } catch {
        /* ignore */
      }
    }
  }

  return (
    <>
      {/* ── Dock：右侧固定按钮列。渐显渐隐（进入防抖 180ms / 渐显后常驻 3s）──
          列排布直接落在 dock 上（竖向 / gap 4px / 右对齐），同 .chat-header-right：
          按钮等宽 30px，flex-end 与 center 视觉等价，用 flex-end 与 header 一致。
          热区 = 这一列按钮的真实盒子，dock 不加任何 padding/margin/inset 撑大。
          .is-shown 由 dockShown 状态驱动，CSS 只负责 opacity/transform 过渡。 */}
      <div
        ref={dockRef}
        className={`desktop-toolbar-dock${dockShown ? ' is-shown' : ''}`}
        onMouseEnter={onDockEnter}
        onMouseLeave={onDockLeave}
      >
        {/* Tool buttons —— 整列同时渐显，不做逐个错峰（见 CSS .is-shown 注释）。
            提示只在 .desktop-toolbar-label 里出：主行名称 + 副行快捷键/说明。
            **不再挂 title** —— 原生灰框会和自定义浮层同时出现（双重提示）。 */}
        {TOOLS.map(tool => (
          <IconButton
            key={tool.mode}
            variant={activeTool === tool.mode ? 'desktop-toolbar-active' : 'desktop-toolbar'}
            label={t(tool.labelKey)}
            nativeTitle={false}
            onClick={() => handleToolClick(tool.mode)}
          >
            <tool.icon size={15} />
            <span className="desktop-toolbar-label">
              <span className="desktop-toolbar-label-name">{t(tool.labelKey)}</span>
              {(tool.hint || tool.detail) && (
                <span className="desktop-toolbar-label-hint">{tool.hint ?? tool.detail}</span>
              )}
            </span>
          </IconButton>
        ))}

        <IconButton
          variant="desktop-toolbar"
          label={t('toolbar.registerApp')}
          nativeTitle={false}
          disabled={loading}
          onClick={async () => {
            setLoading(true)
            try {
              const registered = await tauriInvoke<{ name: string } | null>(
                'desktop_register_application',
              )
              if (registered)
                showAppFeedback(
                  `应用「${registered.name}」已登记，可在工作流中按应用名称选择。`,
                  'success',
                )
            } catch (error) {
              showAppFeedback(`应用登记失败：${String(error)}`, 'error')
            } finally {
              setLoading(false)
            }
          }}
        >
          <IconAppWindow size={15} />
          <span className="desktop-toolbar-label">
            <span className="desktop-toolbar-label-name">{t('toolbar.registerApp')}</span>
            <span className="desktop-toolbar-label-hint">登记未被自动发现的应用</span>
          </span>
        </IconButton>

        {/* ── 工具：内置工具页入口（整页独立界面，不在画布内）──
            列末位；工具页自身有整页壳与关闭按钮，这里只负责打开，不做任何面板内嵌。 */}
        {onOpenTools && (
          <IconButton
            variant="desktop-toolbar"
            label={t('toolbar.tools')}
            nativeTitle={false}
            onClick={() => {
              setActiveTool(null)
              onOpenTools()
            }}
          >
            <IconWrench size={15} />
            <span className="desktop-toolbar-label">
              <span className="desktop-toolbar-label-name">{t('toolbar.tools')}</span>
              <span className="desktop-toolbar-label-hint">PDF / 图片 / 视频 / 音频</span>
            </span>
          </IconButton>
        )}

        {/* ── 鼠标坐标：读数 + 关闭**并排成一组**，且**常驻不受 hover 显隐约束** ──
            为什么必须常驻：整列平时 opacity:0，只在 hover dock 时才显形。若坐标
            读数跟着一起隐藏，用户为了看读数/去别处用坐标，鼠标一离开 dock 就
            什么都看不见了（2026-10-09 大王报障「点击后坐标位置获取不到」），
            关闭钮也一起消失、无从点击。故本组自成一体：只要 activeTool 是
            mouse_pos 就常驻显示，点关闭才收。
            关闭钮与读数同级并排（原实现把它排在列末尾、落在读数「下方」）。 */}
        {activeTool === 'mouse_pos' && (
          <div className="desktop-toolbar-pos-group" role="group" aria-label="鼠标坐标">
            {posError ? (
              // 被拒（典型：automation_busy 任务执行中）时给出可见原因 —— 静默停在
              // (0, 0) 会让人以为「点击无效」，实际是后端资源门拒绝。
              <button
                type="button"
                className="desktop-toolbar-pos desktop-toolbar-pos--error"
                onClick={() => setActiveTool(null)}
                title={`实时坐标不可用：${posError}（点击关闭）`}
              >
                坐标不可用
              </button>
            ) : (
              <button
                type="button"
                className="desktop-toolbar-pos"
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText(`${cursorPos.x},${cursorPos.y}`)
                  } catch {
                    /* ignore */
                  }
                }}
                title="点击复制坐标"
              >
                ({cursorPos.x}, {cursorPos.y})
              </button>
            )}
            <button
              type="button"
              className="desktop-toolbar-pos-close"
              aria-label="关闭鼠标坐标"
              title="关闭鼠标坐标"
              onClick={() => setActiveTool(null)}
            >
              <IconX size={15} />
            </button>
          </div>
        )}
      </div>

      {/* ── Dictionary OCR panel ── */}
      {showOcrDict && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 200,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            // 遮罩归位到弹窗族语义键：原写死 rgba(0,0,0,0.5) 三主题同值，
            // 与设置中心等弹窗宿主（--overlay-bg）不统一。
            background: 'var(--overlay-bg)',
            backdropFilter: 'blur(4px)',
          }}
          onClick={() => setShowOcrDict(false)}
        >
          <div onClick={e => e.stopPropagation()} style={{ width: '90vw', maxWidth: 780 }}>
            <OcrDictionary onClose={() => setShowOcrDict(false)} />
          </div>
        </div>
      )}

      {/* ── Loading overlay (not shown during color picking, handled by panel itself) ── */}
      {loading && pickingSlotIndex === null && (
        <div className="desktop-toolbar-overlay">
          <div className="desktop-toolbar-loading">
            <span className="desktop-toolbar-spinner" />
            处理中...
          </div>
        </div>
      )}

      {/* ── Color picker panel (9 slots) ── */}
      {activeTool === 'color_picker' && pickingSlotIndex === null && !loading && (
        <div
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 200,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            // 遮罩归位到弹窗族语义键（同上：原写死 rgba(0,0,0,0.5)）。
            background: 'var(--overlay-bg)',
            backdropFilter: 'blur(4px)',
          }}
          onClick={() => {
            setActiveTool(null)
            setColorCopiedIndex(null)
          }}
        >
          <div
            onClick={e => e.stopPropagation()}
            style={{
              // 面板底归位弹窗族：原 --glass-bg-soft（α 三主题写死，且带 rgba 硬兜底），
              // 滑块（主题设置 → 界面不透明度 → 控制面板）遍历不到它。与
              // .settings-center-panel / .session-rail-drawer / .wfst-panel / .wcf-content 同一按键。
              background: 'var(--panel-bg)',
              backdropFilter: 'blur(24px)',
              borderRadius: 20,
              padding: 20,
              width: 440,
              maxHeight: '80vh',
              overflow: 'auto',
              border: '1px solid var(--glass-4, rgba(255,255,255,0.08))',
              boxShadow: 'var(--shadow-modal)',
            }}
          >
            {/* Title bar */}
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                marginBottom: 14,
                paddingBottom: 10,
                borderBottom: '1px solid var(--glass-2)',
              }}
            >
              <span
                style={{
                  fontSize: 'var(--fs-h2)',
                  fontWeight: 'var(--fw-semibold)',
                  color: 'var(--spark-primary)',
                }}
              >
                取色器
              </span>
              <button
                onClick={() => {
                  setActiveTool(null)
                  setColorCopiedIndex(null)
                }}
                style={{
                  width: 28,
                  height: 28,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  border: '1px solid var(--glass-2)',
                  background: 'var(--glass-0)',
                  borderRadius: 8,
                  color: 'var(--spark-tertiary)',
                  cursor: 'pointer',
                  fontSize: 16,
                  lineHeight: 1,
                  transition: 'var(--transition-fast)',
                }}
                onMouseEnter={e => {
                  e.currentTarget.style.background = 'var(--glass-3)'
                  e.currentTarget.style.color = 'var(--spark-primary)'
                }}
                onMouseLeave={e => {
                  e.currentTarget.style.background = 'var(--glass-0)'
                  e.currentTarget.style.color = 'var(--spark-tertiary)'
                }}
              >
                <IconX size={14} />
              </button>
            </div>

            {/* 9 slots */}
            <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
              {colorSlots.map((slot, i) => (
                <div
                  key={i}
                  onClick={() => handleColorSlotPick(i)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 10,
                    padding: '8px 10px',
                    borderRadius: 10,
                    cursor: 'pointer',
                    background: 'var(--glass-1)',
                    border: '1px solid var(--glass-2)',
                    transition: 'var(--transition-fast)',
                  }}
                  onMouseEnter={e => {
                    e.currentTarget.style.borderColor = 'var(--accent)'
                  }}
                  onMouseLeave={e => {
                    e.currentTarget.style.borderColor = 'var(--glass-2)'
                  }}
                >
                  {/* Color swatch */}
                  <div
                    style={{
                      width: 32,
                      height: 32,
                      borderRadius: 8,
                      flexShrink: 0,
                      background: slot.filled ? slot.hex : 'var(--glass-2)',
                      border: '1px solid var(--glass-3)',
                    }}
                  />

                  {/* Color info */}
                  <div
                    style={{
                      flex: 1,
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 1,
                      minWidth: 0,
                    }}
                  >
                    <span
                      style={{
                        fontFamily: 'var(--font-mono)',
                        fontSize: 13,
                        color: slot.filled ? 'var(--spark-primary)' : 'var(--spark-muted)',
                      }}
                    >
                      {slot.filled ? slot.hex : `槽位 ${i + 1} — 点击取色`}
                    </span>
                    {slot.filled && (
                      <span
                        style={{
                          fontFamily: 'var(--font-mono)',
                          fontSize: 11,
                          color: 'var(--spark-tertiary)',
                        }}
                      >
                        RGB({slot.rgb.join(', ')})
                      </span>
                    )}
                  </div>

                  {/* Action buttons */}
                  {slot.filled ? (
                    <button
                      onClick={e => {
                        e.stopPropagation()
                        const text = `${slot.hex}  RGB: ${slot.rgb.join(', ')}`
                        navigator.clipboard
                          .writeText(text)
                          .then(() => {
                            setColorCopiedIndex(i)
                            setTimeout(() => setColorCopiedIndex(null), 2000)
                          })
                          .catch(() => {})
                      }}
                      style={{
                        background:
                          colorCopiedIndex === i ? 'rgba(76,175,80,0.15)' : 'var(--glass-0)',
                        border: '1px solid var(--glass-2)',
                        borderRadius: 8,
                        padding: '4px 12px',
                        color: colorCopiedIndex === i ? '#4caf50' : 'var(--spark-tertiary)',
                        cursor: 'pointer',
                        fontSize: 11,
                        transition: 'var(--transition-fast)',
                      }}
                      onMouseEnter={e => {
                        if (colorCopiedIndex !== i) {
                          e.currentTarget.style.background = 'var(--glass-2)'
                          e.currentTarget.style.color = 'var(--spark-secondary)'
                        }
                      }}
                      onMouseLeave={e => {
                        if (colorCopiedIndex !== i) {
                          e.currentTarget.style.background = 'var(--glass-0)'
                          e.currentTarget.style.color = 'var(--spark-tertiary)'
                        }
                      }}
                    >
                      {colorCopiedIndex === i ? '✓' : '复制'}
                    </button>
                  ) : (
                    <span style={{ color: 'var(--spark-muted)', fontSize: 11 }}>空</span>
                  )}
                </div>
              ))}
            </div>

            {/* Clear button */}
            <button
              onClick={() => setColorSlots(initSlots())}
              style={{
                marginTop: 12,
                width: '100%',
                padding: '6px 0',
                background: 'var(--glass-0)',
                border: '1px solid var(--glass-2)',
                borderRadius: 8,
                color: 'var(--spark-tertiary)',
                cursor: 'pointer',
                fontSize: 12,
                transition: 'var(--transition-fast)',
              }}
              onMouseEnter={e => {
                e.currentTarget.style.background = 'var(--glass-1)'
                e.currentTarget.style.color = 'var(--spark-secondary)'
              }}
              onMouseLeave={e => {
                e.currentTarget.style.background = 'var(--glass-0)'
                e.currentTarget.style.color = 'var(--spark-tertiary)'
              }}
            >
              清空所有颜色
            </button>
          </div>
        </div>
      )}

      {/* ── Result popup ── */}
      {result && (
        <div
          className="desktop-toolbar-overlay"
          onClick={() => {
            setResult(null)
            setCopied(false)
          }}
        >
          <div className="desktop-toolbar-result" onClick={e => e.stopPropagation()}>
            <div className="desktop-toolbar-result-header">
              <span className="desktop-toolbar-result-title">{result.title}</span>
              <div style={{ display: 'flex', gap: 6 }}>
                <Button variant="default" size="sm" onClick={handleCopyResult}>
                  {copied ? <IconCheck size={13} /> : <IconCopy size={13} />}
                  {copied ? '已复制' : '复制'}
                </Button>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => {
                    setResult(null)
                    setCopied(false)
                  }}
                >
                  关闭
                </Button>
              </div>
            </div>
            <pre className={`desktop-toolbar-result-content ${result.type}`}>{result.content}</pre>
          </div>
        </div>
      )}
    </>
  )
}
