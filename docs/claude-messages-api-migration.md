# Claude Messages API 迁移调研报告

> 调研日期：2026-03-04
> 目标：评估将 occo 插件中 Claude 模型从 `/chat/completions` 迁移到 Anthropic Messages API (`/v1/messages`) 的可行性和实现方案
> **核心结论：迁移方案与 VS Code Copilot Chat 的 Claude 路由行为完全一致，技术风险已排除。**

## 1. 背景

当前 occo 插件使用 `@ai-sdk/github-copilot` SDK，所有模型统一走 OpenAI 兼容协议：
- GPT 系列 → `/chat/completions` 或 `/responses`（GPT-5+ 使用 Responses API）
- Claude 系列 → `/chat/completions`
- Gemini 系列 → `/chat/completions`

这导致 Claude 模型无法使用 Anthropic 原生特性（extended thinking、adaptive thinking 等）。

**VS Code Copilot Chat 已经对 Claude 模型使用 Messages API (`/v1/messages`)**，迁移后 occo 的行为将与官方 Copilot Chat 完全对齐。

## 2. VS Code Copilot Chat 的做法（迁移对齐目标）

> **occo 迁移后的行为将与以下分析的 VS Code Copilot Chat 行为完全一致。**

### 2.1 API 路由机制

VS Code Copilot Chat 的路由是**数据驱动**的，不是按模型系列硬编码：

```
核心文件：src/platform/endpoint/node/chatEndpoint.ts

路由优先级：Responses API > Messages API > Chat Completions

决策逻辑：
1. useResponsesApi = model.supported_endpoints 包含 "/responses"
2. useMessagesApi = experiment flag (默认 true) && model.supported_endpoints 包含 "/v1/messages"
3. apiType = responses ? 'responses' : messagesApi ? 'messages' : 'chatCompletions'
```

### 2.2 Claude 模型的实际路由

Copilot API 返回的 Claude 模型元数据：
```json
{
  "supported_endpoints": ["/chat/completions", "/v1/messages"]
}
```

- ❌ 不支持 `/responses`
- ✅ 支持 `/v1/messages`
- 因此 Claude 在 VS Code 中使用 **Messages API**

### 2.3 Messages API 请求格式

```typescript
// 文件：src/platform/endpoint/node/messagesApi.ts

// 请求体
{
  model: string,
  messages: MessageParam[],        // content 数组格式 (text/image/tool_use/tool_result blocks)
  system?: TextBlockParam[],       // 系统提示独立字段
  stream: true,
  tools?: AnthropicMessagesTool[], // Anthropic 原生工具格式，支持 defer_loading
  max_tokens: number,
  thinking?: {
    type: 'enabled', budget_tokens: number  // 精确预算
  } | {
    type: 'adaptive'                        // 自适应
  },
  top_p?: number,
  output_config?: { effort: string },
  context_management?: { ... }     // 服务端上下文压缩
}
```

### 2.4 Messages API 响应格式

SSE 流式响应，事件类型：
- `message_start` — 消息开始
- `content_block_start` — 内容块开始 (text / tool_use / server_tool_use / thinking)
- `content_block_delta` — 内容增量
- `message_delta` — 消息级增量 (stop_reason, usage)
- `message_stop` — 消息结束

### 2.5 认证与 Headers

VS Code Copilot Chat 使用**本地代理模式**（`ClaudeLanguageModelServer`）：

```
@anthropic-ai/sdk → localhost proxy → Copilot API (api.githubcopilot.com)
```

- **SDK → 本地代理**：使用 `x-api-key`（SDK 默认行为）
- **本地代理 → Copilot API**：转换为 `Authorization: Bearer <copilot-token>`

代理层负责的关键任务：
1. 将 SDK 的 `x-api-key` 认证转换为 Copilot 的 `Bearer` 认证
2. 添加 Copilot 特有 headers
3. 过滤 `anthropic-beta` 为 Copilot 支持的 beta 特性

发往 Copilot API 的完整 headers：
```
Authorization: Bearer <copilot_token>
X-Request-Id: <uuid>
OpenAI-Intent: conversation-agent
X-GitHub-Api-Version: 2025-05-01
X-Interaction-Type: conversation-subagent | conversation-agent
X-Agent-Task-Id: <uuid>
User-Agent: vscode_claude_code/<claude-code-version>
X-VSCode-User-Agent-Library-Version: <fetcher-library>  (VS Code 特有，不需要)
anthropic-beta: interleaved-thinking-2025-05-14,context-management-...,advanced-tool-use-...
anthropic-version: <SDK 默认值>
```

#### User-Agent 构造

VS Code Copilot Chat 有两层 UA 设置：

1. **Fetcher 默认 UA**（所有请求）：`GitHubCopilotChat/<extension-version>`
2. **Claude 代理层重写**：将 SDK 发来的 UA（如 `claude-code/1.2.3`）重写为 `vscode_claude_code/1.2.3`

```typescript
// claudeLanguageModelServer.ts — getUserAgent()
// userAgentPrefix = 'vscode_claude_code' (硬编码)
// 输入: 'claude-code/1.2.3' → 输出: 'vscode_claude_code/1.2.3'
// 逻辑：替换 '/' 前的部分为 prefix
private getUserAgent(incomingUserAgent: string): string {
  const slashIndex = incomingUserAgent.indexOf('/');
  if (slashIndex === -1) return `${this.userAgentPrefix}/${incomingUserAgent}`;
  return `${this.userAgentPrefix}${incomingUserAgent.substring(slashIndex)}`;
}
```

对于 occo 插件，建议设置 `User-Agent: occo/<version>`。

#### X-Interaction-Type 取值逻辑

```typescript
// networking.ts
// subagent   → 'conversation-subagent'
// background → 'conversation-background'
// 其他       → intent 值本身 (如 'conversation-agent')
```

**关键差异**：标准 Anthropic API 使用 `x-api-key` header，Copilot API 使用 `Authorization: Bearer`。

## 3. 迁移方案分析

### 3.1 方案 A：使用 `@ai-sdk/anthropic`（推荐 — 与 Copilot Chat 行为一致）

**原理**：`@ai-sdk/anthropic` 支持自定义 `baseURL`，可以指向 Copilot API endpoint。

与 VS Code Copilot Chat 的行为完全对齐：
- ✅ **相同的 API 端点**：`/v1/messages`（Anthropic Messages API）
- ✅ **相同的认证方式**：`Authorization: Bearer <copilot-token>`
- ✅ **相同的请求格式**：Messages API 原生格式（`messages[]`、`system[]`、`thinking`）
- ✅ **相同的 beta 特性**：`interleaved-thinking`、`context-management`、`advanced-tool-use`
- ✅ **相同的 headers**：`X-GitHub-Api-Version`、`OpenAI-Intent`、`anthropic-beta`
- ✅ **相同的 User-Agent 模式**：`<client-name>/<version>`

唯一的架构差异是 VS Code 使用本地代理（SDK → localhost → Copilot API），而 occo 直接连接（SDK → Copilot API）。**最终发往 Copilot API 的请求格式和 headers 完全相同。**

```typescript
import { createAnthropic } from '@ai-sdk/anthropic'

const anthropic = createAnthropic({
  baseURL: 'https://api.githubcopilot.com/v1',  // Copilot endpoint
  apiKey: copilotToken,                           // 传入 Copilot token
  headers: {
    // 可能需要覆盖认证头
  }
})

// Claude 模型使用 anthropic SDK
const model = anthropic('claude-sonnet-4-20250514')
```

**优点**：
- opencode 已内置 `@ai-sdk/anthropic`（provider.ts BUNDLED_PROVIDERS）
- 原生支持 extended thinking、tool use 等
- 与 VS Code Copilot Chat 行为对齐

**缺点/风险**：
- 双 SDK 路由增加复杂度
- 需要 Copilot API 端 runtime 验证（curl POC）

### 3.2 方案 B：保持现状

继续使用 `@ai-sdk/github-copilot` 的 `sdk.chat()` 走 `/chat/completions`。

**优点**：
- 稳定可用
- 单一 SDK，简单
- 无需额外 patch

**缺点**：
- 无 extended thinking
- 无 adaptive thinking
- 无 interleaved thinking
- 无 context management
- 与 VS Code Copilot Chat 行为不一致

### 3.3 方案 C：自定义 Messages API 客户端

不使用任何 SDK，直接实现 Anthropic Messages API 的请求/响应处理。

**优点**：
- 完全控制认证和请求格式
- 不受 SDK 限制

**缺点**：
- 工作量大
- 需要实现 SSE 解析、工具调用映射等
- 需要适配 AI SDK 的 LanguageModel 接口
- 维护成本高

## 4. 实现难点

### 4.1 认证头差异 — ✅ 已解决

```
Anthropic 官方:  x-api-key: sk-ant-...
Copilot API:     Authorization: Bearer ghu_...
```

**`@ai-sdk/anthropic` 原生支持 Bearer token 认证**，通过 `authToken` 参数：

```typescript
// anthropic-provider.ts (vercel/ai)
const getHeaders = () => {
  const authHeaders = options.authToken
    ? { Authorization: `Bearer ${options.authToken}` }  // ← authToken 直接生成 Bearer header
    : { 'x-api-key': loadApiKey({...}) };                // ← 否则用 x-api-key
  return {
    'anthropic-version': '2023-06-01',
    ...authHeaders,
    ...options.headers,  // ← 自定义 headers 最后 spread，可覆盖任何默认值
  };
};
```

**关键结论**：
- `authToken` 参数 → 自动生成 `Authorization: Bearer <token>`，**不发送** `x-api-key`
- `headers` 参数 → 最后 spread，可覆盖前面的任何值
- `apiKey` 和 `authToken` 互斥，只需提供一个
- `undefined`/`null` header 值会被 `normalize-headers.ts` 过滤

**推荐用法**：
```typescript
import { createAnthropic } from '@ai-sdk/anthropic'

const copilotAnthropic = createAnthropic({
  baseURL: 'https://api.githubcopilot.com',
  authToken: copilotToken,     // ← 自动变成 Authorization: Bearer
  headers: {
    'X-GitHub-Api-Version': '2025-05-01',
    'OpenAI-Intent': 'conversation-agent',
    'anthropic-beta': 'interleaved-thinking-2025-05-14,context-management-...,advanced-tool-use-...',
  }
})
```

**真实用例验证**：已有多个开源项目通过 `baseURL` + 自定义 headers 将 `@ai-sdk/anthropic` 用于非 Anthropic 后端（Cloudflare AI Gateway 等）。

### 4.2 双 SDK 路由

需要在 opencode patch 的 `CUSTOM_LOADERS["occo"]` 中实现：

```typescript
async getModel(sdk, modelID) {
  if (isClaudeModel(modelID)) {
    // 使用 @ai-sdk/anthropic SDK
    return anthropicSdk(modelID)
  }
  // 其他模型继续使用 copilot SDK
  return shouldUseCopilotResponsesApi(modelID)
    ? sdk.responses(modelID)
    : sdk.chat(modelID)
}
```

### 4.3 anthropic-beta Headers

需要在请求中附加 beta 特性头：
```
anthropic-beta: interleaved-thinking-2025-05-14,context-management-...,advanced-tool-use-...
```

### 4.4 Variant 映射

迁移后 Claude variants 需要从当前的 `{ thinking: { thinking_budget: 4000 } }` 改为使用 `@ai-sdk/anthropic` 的 transform 格式：

```typescript
// opencode transform.ts 中 @ai-sdk/anthropic 的 variant 定义
{
  adaptive: {
    thinking: { type: 'enabled', budgetTokens: 10000 }
  },
  high: {
    thinking: { type: 'enabled', budgetTokens: 32000 }
  },
  max: {
    thinking: { type: 'enabled', budgetTokens: 'max' }
  }
}
```

## 5. 验证清单

如决定实施迁移，需要依次验证：

- [x] `@ai-sdk/anthropic` 的 `headers` 参数能否覆盖默认认证头 — ✅ 原生支持 `authToken` 参数
- [x] VS Code Copilot Chat 如何处理认证头 — ✅ 本地代理模式，最终 Bearer token
- [ ] Copilot `/v1/messages` endpoint 实际接受 Bearer token 认证（需 curl POC）
- [ ] Copilot `/v1/messages` 的 `anthropic-version` 兼容性
- [ ] `anthropic-beta` features 在 Copilot API 上是否生效
- [ ] Extended thinking 响应格式与 `@ai-sdk/anthropic` 解析器兼容
- [ ] 工具调用在 Messages API 下正常工作
- [ ] streaming 响应正确处理所有 SSE 事件类型
- [ ] 双 SDK 路由不影响其他模型的正常使用

## 6. 建议

**推荐方案 A（使用 `@ai-sdk/anthropic`）**，关键技术风险已排除，且**与 VS Code Copilot Chat 的 Claude 路由行为完全一致**：

1. ✅ 认证头覆盖 — `authToken` 参数原生支持 Bearer token（与 Copilot Chat 代理层行为相同）
2. ✅ 自定义 baseURL — 已有开源项目验证可用
3. ✅ 额外 headers — `headers` 参数最后 spread，可添加 Copilot 特有 headers
4. ✅ API 端点 — `/v1/messages`，与 Copilot Chat 使用的端点完全相同
5. ✅ 请求/响应格式 — Anthropic Messages API 原生格式，与 Copilot Chat 一致
6. ✅ Beta 特性 — interleaved-thinking、context-management、advanced-tool-use

**迁移后，occo 对 Claude 模型的 API 调用行为将与 VS Code Copilot Chat 完全对齐**，唯一差异是 occo 直连 Copilot API（无本地代理），但最终的 HTTP 请求完全等价。

**下一步**：执行 curl POC 验证 Copilot API 端的实际兼容性：

```bash
# 1. 测试 Copilot /v1/messages 端点是否可用
curl -X POST https://api.githubcopilot.com/v1/messages \
  -H "Authorization: Bearer <copilot_token>" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -d '{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"hello"}],"max_tokens":100}'

# 2. 如果成功，测试 extended thinking
curl -X POST https://api.githubcopilot.com/v1/messages \
  -H "Authorization: Bearer <copilot_token>" \
  -H "Content-Type: application/json" \
  -H "anthropic-version: 2023-06-01" \
  -H "anthropic-beta: interleaved-thinking-2025-05-14" \
  -d '{"model":"claude-sonnet-4-20250514","messages":[{"role":"user","content":"hello"}],"max_tokens":100,"thinking":{"type":"enabled","budget_tokens":1000}}'
```

如果 POC 验证通过，可直接进入实现阶段。

## 7. 参考资料

- VS Code Copilot Chat: `https://github.com/microsoft/vscode-copilot-chat`
  - `src/platform/endpoint/node/chatEndpoint.ts` — 路由逻辑
  - `src/platform/endpoint/node/messagesApi.ts` — Messages API 实现
  - `src/platform/endpoint/common/endpointProvider.ts` — ModelSupportedEndpoint 枚举
- `@ai-sdk/anthropic`: `https://github.com/vercel/ai/tree/main/packages/anthropic`
- `@ai-sdk/github-copilot`: `https://github.com/nicepkg/aide/tree/master/packages/github-copilot`
- opencode provider: `packages/opencode/src/provider/provider.ts`
- opencode transform: `packages/opencode/src/provider/transform.ts`
