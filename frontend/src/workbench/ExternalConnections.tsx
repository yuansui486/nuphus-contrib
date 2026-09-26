import { useState } from 'react'
import { useLanguage } from '../locales'
import { clients, type Endpoint } from './api'

export function ExternalConnections({ endpoint }: { endpoint: Endpoint | null }) {
  const { lang } = useLanguage()
  const ui = (zh: string, en: string) => (lang === 'zh' ? zh : en)
  const [checking, setChecking] = useState(false)
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null)
  const local = endpoint?.local
  const copy = (text: string) => {
    void navigator.clipboard
      .writeText(text)
      .then(() => setResult({ ok: true, message: ui('已复制', 'Copied') }))
      .catch(error => setResult({ ok: false, message: String(error) }))
  }
  return (
    <section className="wb-dashboard">
      <h1>{ui('外部接入', 'External connections')}</h1>
      <h2>{ui('本地应用接入（推荐）', 'Local application (recommended)')}</h2>
      <p>
        {ui(
          '将下面的配置添加到 Agent 的 MCP 设置，无需填写端口或密钥。连接时会自动启动工作台到托盘，不打开主窗口。',
          'Add this configuration to your Agent’s MCP settings. No port or key is needed. Connecting automatically starts Workbench in the tray without opening the main window.',
        )}
      </p>
      <p role="status">
        {!local
          ? ui('正在检查本地接入…', 'Checking local connection…')
          : !local.available
            ? ui(
                '未找到 MCP 程序，请安装包含 MCP 程序的完整工作台。',
                'MCP executable is missing. Install the complete Workbench package.',
              )
            : local.status === 'listening'
              ? ui('本地服务已就绪 · 全部权限', 'Local service ready · Full access')
              : (local.message ??
                ui(
                  '本地服务尚未就绪，请重启工作台。',
                  'Local service is not ready; restart Workbench.',
                ))}
      </p>
      {local && (
        <div className="wb-connections">
          <div className="wb-row">
            <div>
              <strong>{ui('MCP 程序', 'MCP executable')}</strong>
              <code>{local.executable}</code>
            </div>
            <button onClick={() => copy(local.executable)}>{ui('复制路径', 'Copy path')}</button>
          </div>
          <pre className="wb-mcp-config">{JSON.stringify(local.config, null, 2)}</pre>
          <div className="wb-actions">
            <button
              disabled={!local.available}
              onClick={() => copy(JSON.stringify(local.config, null, 2))}
            >
              {ui('复制 MCP 配置', 'Copy MCP configuration')}
            </button>
            <button
              disabled={!local.available || checking}
              onClick={() => {
                setChecking(true)
                setResult(null)
                void clients<{ ok: boolean; message: string }>('check')
                  .then(check =>
                    setResult(
                      check.ok
                        ? {
                            ok: true,
                            message: ui(
                              'MCP 初始化、工具发现和工作台连接均正常。',
                              'MCP initialization, tool discovery and Workbench connection passed.',
                            ),
                          }
                        : check,
                    ),
                  )
                  .catch(error => setResult({ ok: false, message: String(error) }))
                  .finally(() => setChecking(false))
              }}
            >
              {checking ? ui('正在测试…', 'Testing…') : ui('测试连接', 'Test connection')}
            </button>
          </div>
          <p>
            {ui(
              '同一路径升级后无需重新配置。移动安装位置后，请重新复制配置；更新期间请暂停 MCP 连接，更新完成后重新连接。',
              'No configuration change is needed when upgrading in place. Copy the configuration again if you move the installation. Pause MCP during updates and reconnect afterward.',
            )}
          </p>
        </div>
      )}
      {result && <p role={result.ok ? 'status' : 'alert'}>{result.message}</p>}
      <details className="wb-http-connection">
        <summary>
          {ui('高级接入：HTTP / Streamable HTTP', 'Advanced: HTTP / Streamable HTTP')}
        </summary>
        <p>
          {endpoint?.status === 'listening'
            ? ui('HTTP 服务已启动', 'HTTP service running')
            : (endpoint?.message ??
              ui(
                'HTTP 服务未就绪；不影响本地 MCP。',
                'HTTP is not ready; local MCP is independent.',
              ))}
        </p>
        {endpoint?.status === 'listening' && (
          <div className="wb-connections">
            {[
              ['MCP', endpoint.mcp_url],
              ['HTTP API', endpoint.url ? `${endpoint.url}/api/v1` : undefined],
            ].map(
              ([label, url]) =>
                url && (
                  <div className="wb-row" key={label}>
                    <div>
                      <strong>{label}</strong>
                      <code>{url}</code>
                    </div>
                    <button onClick={() => copy(url)}>{ui('复制地址', 'Copy address')}</button>
                  </div>
                ),
            )}
            <p>
              {ui(
                'HTTP 可先 GET /api/v1/discover 查看接口。显式使用 URL 的客户端仍需在端口改变时修改地址。',
                'Use GET /api/v1/discover for the HTTP API. URL-based clients still need their address updated if the port changes.',
              )}
            </p>
          </div>
        )}
      </details>
      <p>
        {ui(
          '本机 Agent 可使用全部项目、画布、工作流、定时任务和自动化能力，不需要配置内置模型。断开 MCP 不会停止工作台或取消运行。仅向可信的本机程序提供此配置。',
          'Local Agents can use all projects, canvases, workflows, schedules and automation without configuring an internal model. Disconnecting MCP does not stop Workbench or cancel runs. Share this configuration only with trusted local applications.',
        )}
      </p>
    </section>
  )
}
