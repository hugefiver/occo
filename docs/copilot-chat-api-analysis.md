# VS Code Copilot Chat 官方插件 API 请求行为分析

> 分析日期：2026-03-27
> 对比版本：OCCO 模拟 v0.38.2 vs 官方 v0.36.0 ~ v0.42.x
> 源码仓库：microsoft/vscode-copilot-chat（MIT，2025年12月开源）

## 一、版本演进时间线

| 版本      | VS Code 引擎 | 关键变化                                                                                                                                                                                                  |
| --------- | ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| v0.36.0   | ^1.109.0     | 最早可获取的公开版本。单一 HTTP 路径，X-Initiator 在 chatMLFetcher.ts，X-Interaction-Type 无条件设置，thinking 仅 enabled/disabled                                                                        |
| v0.37.0   | ^1.109.0     | 同 v0.36.0，增加 context_management 字段                                                                                                                                                                  |
| v0.37.8/9 | ^1.109.0     | 同上，最后一个单一 HTTP 路径版本                                                                                                                                                                          |
| v0.38.0   | ^1.110.0     | **重大重构**：拆分 HTTP/WebSocket 双路径，X-Interaction-Type 改为有条件，新增 X-Agent-Task-Id，thinking 增加 adaptive 类型，新增 output_config.effort，新增 conversation-subagent/conversation-background |
| v0.38.2   | ^1.110.0     | 同 v0.38.0 结构                                                                                                                                                                                           |
| v0.39.0   | ^1.111.0     | X-Interaction-Type 回归无条件（agentInteractionType 逻辑末尾返回 intent 而非 undefined），X-Agent-Task-Id 也变为无条件                                                                                    |
| v0.40.0   | ^1.111.0     | capabilities 动态化（supportsAdaptiveThinking 等从 modelMetadata 获取），Gemini function calling mode 实验                                                                                                |
| v0.41.0   | ^1.111.0     | Messages API 思维控制从 `!disableThinking` 改为 `enableThinking`（正向控制），新增 `forceExtendedThinking` 实验，Responses API effort 来源从实验配置改为 `options.reasoningEffort`                        |
| v0.41.1/2 | ^1.111.0     | 与 v0.41.0 所有关键文件（networking.ts, chatEndpoint.ts, messagesApi.ts, responsesApi.ts）**完全一致**                                                                                                    |
| v0.42.x   | ^1.111.0     | IEndpointBody 新增 `prompt_cache_key` 字段，Responses API 支持 prompt 缓存（`ResponsesApiPromptCacheKeyEnabled` 实验），effort 默认值改为 `'medium'`                                                      |
| main      | ^1.111.0     | 与 v0.42.0 基本一致，version 仍为 0.42.0。X-Initiator 确认仍存在于 chatMLFetcher.ts line 1255                                                                                                             |

> **X-Initiator 全版本确认**：通过 grep.app 搜索 main 分支确认，`X-Initiator` 从 v0.36.0 到 main 一直存在于 chatMLFetcher.ts 的 additionalHeaders 中（line 1255），**从未被移除**。networking.ts HTTP 路径从未有过 X-Initiator。

## 二、HTTP Headers 详细分析

### 2.1 官方请求头架构

官方插件的 header 分布在两个层级：

**networking.ts — `networkRequest()` 核心头：**

```typescript
const headers: ReqHeaders = {
  Authorization: `Bearer ${secretKey}`, // Copilot session token
  "X-Request-Id": requestId, // UUID
  "X-Interaction-Type": intent, // v0.37.x 无条件; v0.38.x 有条件; v0.39.x+ 无条件
  "OpenAI-Intent": intent, // 动态，来自 locationToIntent()
  "X-GitHub-Api-Version": "2025-05-01",
  ...additionalHeaders, // 来自 chatMLFetcher.ts
  ...(endpoint.getExtraHeaders ? endpoint.getExtraHeaders(location) : {}),
};
// v0.38.0+ 新增（有条件或无条件，取决于版本）：
headers["X-Agent-Task-Id"] = requestId;
```

**chatMLFetcher.ts — 作为 additionalHeaders 传入：**

```typescript
const additionalHeaders = {
  "X-Interaction-Id": this._interactionService.interactionId, // session 级 UUID
  "X-Initiator": userInitiatedRequest ? "user" : "agent",
};
// 有条件：
if (vision) additionalHeaders["Copilot-Vision-Request"] = "true";
```

**HeaderContributor — 通过 baseFetchFetcher.ts 注入：**

- `User-Agent: GitHubCopilotChat/${version}`
- `Editor-Version: vscode/${vsCodeVersion}`
- `Editor-Plugin-Version: copilot-chat/${version}`
- `Copilot-Integration-Id: vscode-chat`

### 2.2 locationToIntent 映射（所有版本一致）

```typescript
Panel           → 'conversation-panel'      // 侧边栏聊天，无工具
Agent           → 'conversation-agent'      // Agent 模式，有工具调用
Editor          → 'conversation-inline'     // Ctrl+I 内联编辑
EditingSession  → 'conversation-edits'      // Copilot Edits（多文件编辑）
Terminal        → 'conversation-terminal'
Notebook        → 'conversation-notebook'
Other           → 'conversation-other'
ResponsesProxy  → 'responses-proxy'
MessagesProxy   → 'messages-proxy'
```

OpenAI-Intent **始终**来自此映射。X-Interaction-Type 在不同版本中行为不同：

- v0.37.x: `X-Interaction-Type = intent`（无条件，等于 OpenAI-Intent）
- v0.38.x+: `X-Interaction-Type = agentInteractionType`，可被覆盖为 `conversation-subagent` 或 `conversation-background`（见 §2.3）
- v0.39.x+: 同上，但始终有值（不会 undefined）

> 注意：`conversation-subagent` 和 `conversation-background` **不是** locationToIntent 的返回值，而是 agentInteractionType 逻辑根据 `requestKindOptions.kind` 生成的。ChatLocation 枚举中没有 Subagent 或 Background。

### 2.3 agentInteractionType 逻辑演进

**v0.38.x（有条件）：**

```typescript
const agentInteractionType =
  kind === "subagent"
    ? "conversation-subagent"
    : kind === "background"
      ? "conversation-background"
      : intent === "conversation-agent"
        ? intent
        : undefined; // ← 可能 undefined
```

**v0.39.x+（无条件）：**

```typescript
const agentInteractionType =
  kind === "subagent"
    ? "conversation-subagent"
    : kind === "background"
      ? "conversation-background"
      : intent === "conversation-agent"
        ? intent
        : intent; // ← 始终有值
```

### 2.4 X-Initiator 判断逻辑

官方有两处 `userInitiatedRequest` 判断：

**toolCallingLoop.ts（主路径）：**

```typescript
userInitiatedRequest: (iterationNumber === 0 &&
  !isContinuation &&
  !subAgentInvocationId) ||
  stopHookUserInitiated;
```

含义：首次迭代 + 非续接 + 非子Agent调用 → user；工具调用后续轮次 → agent

**subagent 的特殊行为**：由于 `subAgentInvocationId` 存在，subagent 的 `userInitiatedRequest` **始终为 false**，即 `X-Initiator` 永远是 `agent`。

**langModelServer.ts（Messages API 代理路径）：**

```typescript
const userInitiatedRequest =
  parsedRequest.messages.at(-1)?.role === Raw.ChatRole.User;
```

含义：最后一条消息是用户发的 → user

**OCCO 当前逻辑：**

```javascript
isAgent = last?.role !== "user" || userMessageCount > 1;
```

含义：最后一条消息非用户 → agent；用户消息超过1条 → agent

### 2.5 X-Initiator 与 X-Interaction-Type 的关系

这是两个**正交维度**：

- `X-Initiator: user/agent` → 每请求计费标识（user = premium，agent = free）
- `X-Interaction-Type: agent/subagent/background` → 请求类型分类

组合示例：

| 场景                   | X-Initiator | X-Interaction-Type      |
| ---------------------- | ----------- | ----------------------- |
| 用户首次提问           | user        | conversation-agent      |
| 工具调用后续           | agent       | conversation-agent      |
| 子Agent调用            | agent       | conversation-subagent   |
| 后台任务（标题生成等） | agent       | conversation-background |

### 2.6 Subagent 请求传播机制

`requestKindOptions` 决定 `X-Interaction-Type` 取值。关键问题：subagent 的后续请求（工具调用 follow-up）是否仍然使用 `conversation-subagent`？

**答案：是，所有迭代都使用 `conversation-subagent`。**

源码位于 `defaultIntentRequestHandler.ts` 的 `DefaultToolCallingLoop.fetch()`：

```typescript
protected override async fetch(opts: ToolCallingLoopFetchOptions, token: CancellationToken): Promise<ChatResponse> {
    return this.options.invocation.endpoint.makeChatRequest2({
        ...opts,
        // ...
        requestKindOptions: this.options.request.subAgentInvocationId
            ? { kind: 'subagent' }
            : undefined,
    }, token);
}
```

`this.options.request.subAgentInvocationId` 在 loop 创建时设置，不会改变。`fetch()` 每次迭代都会被调用，因此 subagent 的**所有请求**都携带 `{ kind: 'subagent' }`。

同样，`searchSubagentToolCallingLoop.ts` 和 `executionSubagentToolCallingLoop.ts` 直接硬编码 `requestKindOptions: { kind: 'subagent' }`。

**完整的 intent 分配矩阵：**

| 场景 | requestKindOptions | X-Interaction-Type | X-Initiator |
|------|-------------------|-------------------|-------------|
| 普通对话（首次） | `undefined` | `conversation-agent`¹ | `user` |
| 普通对话（tool follow-up） | `undefined` | `conversation-agent`¹ | `agent` |
| Subagent（首次） | `{ kind: 'subagent' }` | `conversation-subagent` | `agent`² |
| Subagent（tool follow-up） | `{ kind: 'subagent' }` | `conversation-subagent` | `agent`² |
| Background（标题生成等） | `{ kind: 'background' }` | `conversation-background` | — |

¹ v0.37.x 无条件；v0.38.x 有条件（仅当 intent=conversation-agent 时设置）；v0.39.x+ 无条件回归
² subagent 的 `userInitiatedRequest` 始终为 false（因为 `subAgentInvocationId` 存在，见 §2.4 toolCallingLoop.ts 判断逻辑）

**Subagent 的调用入口**（v0.38.0+ 新增）：

- `searchSubagentTool.ts` — 搜索子Agent，创建 `SearchSubagentToolCallingLoop`
- `executionSubagentTool.ts` — 执行子Agent，创建 `ExecutionSubagentToolCallingLoop`
- 两者都生成独立的 `subAgentInvocationId`（UUID），用于 trajectory 追踪和父子关联

**OCCO 对应**：OCCO `chat.headers` hook 中 `parentID` 存在时设置 `openai-intent: conversation-agent` 和 `x-interaction-type: conversation-agent`。严格来说应为 `conversation-subagent`，但由于计费行为由 `X-Initiator` 控制（已设为 `agent`），实际影响有限。

### 2.7 各版本 Header 存在性总结

| Header                 | v0.37.x   | v0.38.x HTTP | v0.38.x WS | v0.39.x+ HTTP | v0.39.x+ WS |
| ---------------------- | --------- | ------------ | ---------- | ------------- | ----------- |
| Authorization          | ✅        | ✅           | ✅         | ✅            | ✅          |
| X-Request-Id           | ✅        | ✅           | ✅         | ✅            | ✅          |
| OpenAI-Intent          | ✅ 动态   | ✅ 动态      | ✅ 动态    | ✅ 动态       | ✅ 动态     |
| X-GitHub-Api-Version   | ✅        | ✅           | ✅         | ✅            | ✅          |
| X-Interaction-Type     | ✅ 无条件 | ✅ 有条件    | ✅ 有条件  | ✅ 无条件     | ✅ 无条件   |
| X-Initiator            | ✅        | ❌           | ✅         | ❌            | ✅          |
| X-Interaction-Id       | ✅        | ❌           | ✅         | ❌            | ✅          |
| X-Agent-Task-Id        | ❌        | ✅ 有条件    | ✅ 有条件  | ✅ 无条件     | ✅ 无条件   |
| Copilot-Vision-Request | ✅ 有条件 | ✅ 有条件    | ✅ 有条件  | ✅ 有条件     | ✅ 有条件   |
| User-Agent             | ✅        | ✅           | ✅         | ✅            | ✅          |
| Editor-Version         | ✅        | ✅           | ✅         | ✅            | ✅          |
| Editor-Plugin-Version  | ✅        | ✅           | ✅         | ✅            | ✅          |
| Copilot-Integration-Id | ✅        | ✅           | ✅         | ✅            | ✅          |

> 说明：HTTP = networking.ts 路径；WS = chatMLFetcher.ts WebSocket 路径（v0.38.0+ 新增）

## 三、思维链/推理配置

### 3.1 Claude 模型 — ChatCompletions 路径（`/chat/completions`）

OCCO 使用此路径。官方通过 `chatEndpoint.ts` 的 `customizeCapiBody()` 设置。

**请求体字段**：`body.thinking_budget = N`（整数）

**所有版本一致的逻辑（v0.36.0 ~ v0.42.x）：**

```typescript
// chatEndpoint.ts → customizeCapiBody()
if (
  isAnthropicFamily(this) &&
  !options.disableThinking &&
  isConversationAgent
) {
  const thinkingBudget = this._getThinkingBudget();
  if (thinkingBudget) body.thinking_budget = thinkingBudget;
}
```

**`_getThinkingBudget()` 计算方式：**

```typescript
const configuredBudget = getExperimentConfig(ConfigKey.AnthropicThinkingBudget);
// 默认值：16000
const normalizedBudget =
  configuredBudget > 0
    ? Math.max(1024, configuredBudget) // 最小 1024
    : undefined;
return normalizedBudget
  ? Math.min(32000, maxOutputTokens - 1, normalizedBudget) // 上限 32000
  : undefined;
```

**启用条件**：`location === ChatLocation.Agent`（即 intent = `conversation-agent`）

- `conversation-panel` 模式下**不发送** thinking_budget
- 这是 OCCO 之前用 `conversation-panel` 时触发 466 错误的根因

### 3.2 Claude 模型 — Messages API 路径（`/v1/messages`）

OCCO **未使用**此路径，记录备参考。v0.38.0+ 引入。

**请求体**：

```typescript
// messagesApi.ts → createMessagesRequestBody()
// v0.37.x: 仅 enabled + budget_tokens
thinking: thinkingBudget ? { type: 'enabled', budget_tokens: thinkingBudget } : undefined

// v0.38.0+: 增加 adaptive 模式
if (endpoint.supportsAdaptiveThinking) {
    thinkingConfig = { type: 'adaptive' };
} else if (endpoint.maxThinkingBudget && endpoint.minThinkingBudget) {
    thinkingConfig = { type: 'enabled', budget_tokens: computed };
}
// adaptive 模式可附带 effort 级别
...(effort ? { output_config: { effort } } : {})  // effort: 'low'|'medium'|'high'
```

**额外头（getExtraHeaders，仅 Messages API）：**

- `anthropic-beta: interleaved-thinking-2025-05-14`（支持交错思维）
- `anthropic-beta: context-management-2025-06-27`（上下文编辑）
- `anthropic-beta: advanced-tool-use-2025-11-20`（高级工具）
- `X-Model-Provider-Preference`（如配置了供应商偏好）
- `capi-beta-1: true`（不支持交错思维的模型）

**IEndpointBody 类型演进：**

```typescript
// v0.36.0 ~ v0.37.x
thinking?: { type: 'enabled' | 'disabled'; budget_tokens?: number };

// v0.38.0+
thinking?: { type: 'enabled' | 'disabled' | 'adaptive'; budget_tokens?: number };
output_config?: { effort?: 'low' | 'medium' | 'high' };

// v0.42.x+ 新增
prompt_cache_key?: string;
```

**Messages API 思维控制逻辑演进：**

| 版本    | 启用条件                | adaptive 支持 | effort 控制                   | forceExtendedThinking |
| ------- | ----------------------- | ------------- | ----------------------------- | --------------------- |
| v0.37.x | `!disableThinking`      | ❌            | ❌                            | ❌                    |
| v0.38.x | `!disableThinking`      | ✅            | ✅ (adaptive 模型)            | ❌                    |
| v0.40.0 | `!disableThinking`      | ✅ (动态化)   | ✅ (adaptive 模型)            | ❌                    |
| v0.41.x | `enableThinking` (正向) | ✅            | ✅ (adaptive 模型, 带验证)    | ✅                    |
| v0.42.x | `enableThinking`        | ✅            | ✅ (仅 adaptive + type match) | ✅                    |
| main    | `enableThinking`        | ✅            | ✅ (仅 adaptive + type match) | ✅                    |

**v0.41.x 新增细节：**

```typescript
// enableThinking 替代 !disableThinking（正向控制）
if (options.enableThinking) { ... }

// forceExtendedThinking 实验：强制使用 extended thinking 而非 adaptive
const forceExtended = experimentService.getConfig(ConfigKey.AnthropicForceExtendedThinking);
if (endpoint.supportsAdaptiveThinking && !forceExtended) {
    thinkingConfig = { type: 'adaptive' };
} else {
    thinkingConfig = { type: 'enabled', budget_tokens: computed };
}

// effort 仅在 adaptive 模式下发送
const effort = endpoint.supportsAdaptiveThinking ? getConfig(AnthropicThinkingEffort) : undefined;
```

**v0.42.x 进一步细化：**

```typescript
// effort 增加 type 检查
const effort =
  endpoint.supportsAdaptiveThinking && thinkingConfig?.type === "adaptive"
    ? getConfig(AnthropicThinkingEffort)
    : undefined;
```

### 3.3 GPT 模型 — Responses API 路径（`/responses`）

**请求体（responsesApi.ts）：**

```typescript
const effort = effortConfig === "default" ? "medium" : effortConfig;
const summary =
  summaryConfig === "off" || shouldDisableReasoningSummary
    ? undefined
    : summaryConfig;

body.reasoning = {
  ...(effort ? { effort } : {}),
  ...(summary ? { summary } : {}),
};
body.include = ["reasoning.encrypted_content"];
body.text = verbosity ? { verbosity } : undefined; // 'low'|'medium'|'high'
body.truncation = useResponsesApiTruncation ? "auto" : "disabled";
body.store = false;
```

**`shouldDisableReasoningSummary`**：`gpt-5.3-codex-spark-preview` 禁用 summary

**reasoning effort 来源演进：**

| 版本    | effort 来源                                                           | 默认值                            |
| ------- | --------------------------------------------------------------------- | --------------------------------- |
| v0.37.x | `configService.getExperimentBasedConfig(ResponsesApiReasoningEffort)` | 'medium' (当 config='default' 时) |
| v0.38.x | 同上                                                                  | 同上                              |
| v0.39.x | 同上                                                                  | 同上                              |
| v0.40.0 | 同上                                                                  | 同上                              |
| v0.41.x | `options.reasoningEffort`（从调用方传入）                             | 无默认                            |
| v0.42.x | `options.reasoningEffort \|\| 'medium'`                               | 'medium'                          |
| main    | `options.reasoningEffort`                                             | 无默认                            |

**v0.42.x 新增 prompt_cache_key：**

```typescript
// responsesApi.ts — v0.42.x+
if (experimentService.getConfig(ConfigKey.ResponsesApiPromptCacheKeyEnabled)) {
  body.prompt_cache_key = `${conversationId}:${endpoint.family}`;
}
```

**reasoning summary 默认值变化：**

- v0.37.x ~ v0.41.x: `ResponsesApiReasoningSummary` 实验配置控制
- main: 默认值改为 `'detailed'`

## 四、API 路由与端点

### 4.1 端点选择逻辑

官方 `ModelSupportedEndpoint` 枚举：

```typescript
ChatCompletions = "/chat/completions"; // 默认
Responses = "/responses"; // GPT 推理模型
WebSocketResponses = "ws:/responses"; // WebSocket 变体
Messages = "/v1/messages"; // Anthropic Messages API
```

路由优先级：

1. `/v1/messages`：`UseAnthropicMessagesApi` 实验开启 且 模型 supported_endpoints 包含 `/v1/messages`
2. `/responses`：模型不支持 `/chat/completions` 但支持 `/responses`
3. `/chat/completions`：默认

**OCCO 路由**（通过 opencode 的 `shouldUseCopilotResponsesApi()`）：

- GPT-5+（非 mini）→ `sdk.responses()` → `/responses`
- 其他所有 → `sdk.chat()` → `/chat/completions`
- 不支持 `/v1/messages`

### 4.2 关键 URL

| 用途            | URL                                                | 说明                                       |
| --------------- | -------------------------------------------------- | ------------------------------------------ |
| Token 交换      | `https://api.github.com/copilot_internal/v2/token` | GitHub OAuth token → Copilot session token |
| 默认 API        | `https://api.githubcopilot.com`                    | 可被 token 响应中的 `endpoints.api` 覆盖   |
| OAuth Client ID | `Iv1.b507a08c87ecfe98`                             | VS Code Copilot Chat 的 GitHub OAuth App   |

### 4.3 认证流程

1. 用户通过 GitHub OAuth 获得 `github_token`
2. 插件用 `github_token` 请求 `/copilot_internal/v2/token`
3. 服务端返回 Copilot session token（HMAC 签名，有过期时间）
4. 后续 API 请求使用 `Authorization: Bearer ${copilot_session_token}`
5. Token 响应包含 `endpoints.api`（动态 API 地址）和过期时间

**认证头差异**：

- 官方 token 请求：`Authorization: token ${githubToken}`
- OCCO token 请求：`Authorization: Bearer ${info.refresh}`
- 两者均有效

## 五、计费模型

### 5.1 客户端计费判断

官方客户端通过模型元数据判断计费：

```typescript
IChatModelMetadata.billing: {
    is_premium: boolean;       // 是否为 premium 模型
    multiplier: number;        // 计费倍率
    restricted_to?: string[];  // 限制使用的订阅类型
}
```

配额头：`x-quota-snapshot-premium_models` / `x-quota-snapshot-premium_interactions`

### 5.2 服务端计费判断

服务端基于 `X-Initiator` 头决定计费：

- `X-Initiator: user` → premium 请求（消耗用户配额）
- `X-Initiator: agent` → free 请求（工具调用后续，不消耗配额）

来源：coder/mux 开源项目明确注释了此行为。

### 5.3 OCCO 计费相关发现

- v0.40.0 版本号 + 无 X-Initiator → 每个工具调用都被计为 premium → 快速消耗配额
- v0.38.2 版本号 + X-Initiator: agent → 工具调用后续免费
- 必须保留 X-Initiator 头，即使官方 v0.39.x+ HTTP 路径不再发送

## 六、OCCO 当前实现详情

### 6.1 版本号

```javascript
const HEADERS = {
  "User-Agent": "GitHubCopilotChat/0.38.2",
  "Editor-Version": "vscode/1.110.1",
  "Editor-Plugin-Version": "copilot-chat/0.38.2",
  "Copilot-Integration-Id": "vscode-chat",
  "X-GitHub-Api-Version": "2025-05-01",
};
```

### 6.2 动态请求头

```javascript
const intent = "conversation-agent";
const headers = {
  ...init.headers,
  ...HEADERS,
  Authorization: `Bearer ${info.access}`,
  "OpenAI-Intent": intent,
  "X-Interaction-Type": intent,
  "X-Initiator": isAgent ? "agent" : "user",
  "X-Request-Id": requestId,
  "X-Agent-Task-Id": requestId,
};
if (isVision) headers["Copilot-Vision-Request"] = "true";
```

### 6.3 chat.headers Hook

```javascript
"chat.headers": async (incoming, output) => {
    // 每 session 生成稳定 UUID
    output.headers["x-interaction-id"] = interactionIds.get(incoming.sessionID);
    // 子Agent 会话覆盖为 agent
    if (session.data?.parentID) {
        output.headers["x-initiator"] = "agent";
        output.headers["openai-intent"] = "conversation-agent";
        output.headers["x-interaction-type"] = "conversation-agent";
    }
};
```

### 6.4 isAgent 判断

```javascript
// 默认 isAgent = true
// Completions API (body.messages):
isAgent = last?.role !== "user" || userMessageCount > 1;
// Responses API (body.input): 同上逻辑
```

### 6.5 Claude 思维链变体

```javascript
// Opus 4.5+ 默认发送 thinking_budget（通过 model.options）
options: { thinking_budget: 16000 }

// 变体定义
CLAUDE_OPUS_VARIANTS:   { thinking: {thinking_budget:16000}, max: {thinking_budget:32000} }
CLAUDE_LOWER_VARIANTS:  { thinking: {thinking_budget:16000} }
CLAUDE_SONNET4_VARIANTS: { thinking: {thinking_budget:15999} }  // maxOutputTokens=16000, cap=16000-1
```

SDK 流转路径：`model.variants[name]` → `mergeDeep(model.options, variant)` → `ProviderTransform.providerOptions()` → `{ copilot: { thinking_budget: N } }` → SDK 写入 `body.thinking_budget = N`

### 6.6 GPT 推理变体

```javascript
GPT_REASONING_VARIANTS: {
  (low, medium, high);
} // 无默认
GPT_REASONING_VARIANTS_XHIGH: {
  (low, medium, high, xhigh);
} // 无默认
GPT_CODEX_VARIANTS: {
  (high, xhigh);
} // 无默认
```

所有变体包含：`reasoningSummary: "auto"`, `include: ["reasoning.encrypted_content"]`

## 七、OCCO 与官方 v0.38.x 完整差异清单

### 7.1 已对齐 ✅

| 项目                   | 说明                                                                                            |
| ---------------------- | ----------------------------------------------------------------------------------------------- |
| 所有静态头             | User-Agent, Editor-Version, Editor-Plugin-Version, Copilot-Integration-Id, X-GitHub-Api-Version |
| Authorization          | 都使用 Copilot session token                                                                    |
| OpenAI-Intent          | conversation-agent（opencode 等同于 Agent 模式）                                                |
| X-Interaction-Type     | 无条件设置为 intent                                                                             |
| X-Initiator            | user/agent 判断（逻辑略有差异）                                                                 |
| X-Interaction-Id       | session 级 UUID                                                                                 |
| X-Request-Id           | 每请求 UUID                                                                                     |
| X-Agent-Task-Id        | 每请求 UUID                                                                                     |
| Copilot-Vision-Request | 有条件检测 image_url                                                                            |
| Claude thinking_budget | ChatCompletions 路径，默认 16000，最大 32000                                                    |
| GPT reasoning 结构     | effort + summary + include                                                                      |
| Token 交换流程         | /copilot_internal/v2/token                                                                      |
| API 动态路由           | endpoints.api                                                                                   |

### 7.2 存在差异 ⚠️

| 项目                                | 官方行为                | OCCO 行为               | 影响                         |
| ----------------------------------- | ----------------------- | ----------------------- | ---------------------------- |
| X-Initiator 多轮判断                | 新迭代 → user           | userCount>1 → agent     | 低，可能更省配额             |
| OpenAI-Intent 动态性                | locationToIntent() 动态 | 固定 conversation-agent | 低，opencode 本质是 Agent    |
| GPT 默认 reasoning effort           | 始终发送 medium         | 仅变体选择时发送        | 中，可能影响默认推理质量     |
| body.text.verbosity                 | 模型级别配置            | 缺少                    | 低                           |
| body.truncation                     | 'auto' 或 'disabled'    | 缺少                    | 低                           |
| body.store                          | false                   | 缺少                    | 低                           |
| X-VSCode-User-Agent-Library-Version | 存在                    | 缺少                    | 低，服务端不依赖             |
| Messages API                        | 支持 /v1/messages       | 不支持                  | 低，ChatCompletions 功能等价 |

## 八、已知问题与历史经验

### 8.1 466 错误（ClientNotSupported）

- **原因**：服务端根据版本号判断客户端能力，不支持的版本返回 466
- **chatMLFetcher.ts line 1546**：`ChatFailKind.ClientNotSupported`
- **v0.37.9 版本号触发 466**：已确认，改为 v0.38.2 后解决
- **conversation-panel + thinking_budget**：Panel 模式下发送 thinking_budget 可能触发 466

### 8.2 计费异常（v0.40.0 版本号）

- v0.40.0 HTTP 路径不含 X-Initiator
- 当时 OCCO 使用 `conversation-subagent` 作为所有请求的 intent
- 结果：每个工具调用都被计为 premium，快速消耗配额
- 解决：回退到 v0.38.2 版本号 + 保留 X-Initiator

### 8.3 Intent 变迁历史

| 时期           | OCCO Intent           | 结果                                |
| -------------- | --------------------- | ----------------------------------- |
| 早期 (v0.35.0) | conversation-edits    | 正常                                |
| 中期 (v0.38.2) | conversation-panel    | 466 错误 + 无法发送 thinking_budget |
| v0.39 尝试     | conversation-subagent | 计费异常                            |
| 回退           | conversation-panel    | 466                                 |
| 当前 (v0.38.2) | conversation-agent    | ✅ 正常                             |

## 九、升级注意事项

### 9.1 升级到 v0.39.x+ 的风险

1. **必须保留 X-Initiator**：v0.39.x+ HTTP 路径不再发送此头，但服务端仍依赖它做计费判断
2. **X-Agent-Task-Id 变为无条件**：OCCO 已是无条件，无需改动
3. **X-Interaction-Type 变为无条件**：OCCO 已是无条件，无需改动

**v0.39.x+ networking.ts 的关键变化：**

```typescript
// v0.38.x: agentInteractionType 可能为 undefined
const agentInteractionType = ... intent === 'conversation-agent' ? intent : undefined;
if (agentInteractionType) {
    headers['X-Interaction-Type'] = agentInteractionType;  // 有条件
    headers['X-Agent-Task-Id'] = requestId;                // 有条件
}

// v0.39.x+: agentInteractionType 始终有值
const agentInteractionType = ... intent === 'conversation-agent' ? intent : intent;
headers['X-Interaction-Type'] = agentInteractionType;      // 无条件
headers['X-Agent-Task-Id'] = requestId;                    // 无条件
```

**v0.39.x+ chatMLFetcher.ts（WS 路径）仍保留：**

- `X-Initiator: user/agent`（WS 路径未移除）
- `X-Interaction-Id`（WS 路径未移除）

**升级到 v0.39.x+ 版本号时 OCCO 需要的改动：**

- Editor-Version 必须改为 vscode/1.111.x
- 其余 header 行为 OCCO 已兼容（无条件设置所有头）
- 关键：**不能移除 X-Initiator**，否则计费异常

### 9.2 升级到 v0.40.0+ 的额外考虑

v0.40.0 引入了模型元数据动态化：

- `supportsAdaptiveThinking`、`minThinkingBudget`、`maxThinkingBudget` 改为从 `modelMetadata.capabilities.supports.*` 动态获取（而非硬编码）
- 新增 `modelProvider` 字段，来自 `modelMetadata.vendor`
- `getExtraHeaders()` 判断条件从 `modelSupportsInterleavedThinking(model)` 改为 `!this.supportsAdaptiveThinking`（取反逻辑）
- chatEndpoint.ts `customizeCapiBody()` 的 Claude thinking_budget 逻辑**未变**（ChatCompletions 路径不受影响）

**对 OCCO 的影响**：

- OCCO 的模型定义是硬编码的，不受服务端动态化影响
- ChatCompletions 路径的 thinking_budget 行为所有版本一致
- 如需支持 Messages API，需要实现 adaptive thinking 逻辑

### 9.3 升级到 v0.41.x 的额外考虑

v0.41.x 的关键变化集中在思维链控制和推理参数来源：

**Messages API 变化（OCCO 未使用，备记录）：**

- `enableThinking`（正向）替代 `!disableThinking`（双重否定）
- 新增 `forceExtendedThinking` 实验：强制使用 extended thinking 而非 adaptive
- effort 级别现在从调用方 `options.reasoningEffort` 获取，而非实验配置

**Responses API 变化（影响 GPT 模型）：**

- `effort` 来源从 `experimentConfig` 改为 `options.reasoningEffort`
- 无默认值（v0.42.x 才恢复默认 'medium'）

**Header 变化：** 无。与 v0.40.0 完全一致。

**v0.41.0/v0.41.1/v0.41.2 一致性**：三个子版本在所有关键文件（networking.ts, chatEndpoint.ts, messagesApi.ts, responsesApi.ts）上**完全一致**。

**对 OCCO 的影响**：

- ChatCompletions 路径（Claude）无变化
- OCCO 的 GPT 变体通过 SDK 注入 `reasoningSummary` 和 `include`，不受来源变化影响
- 无需代码改动，仅需更新版本号

### 9.4 升级到 v0.42.x 的额外考虑

v0.42.x 唯一显著新增：**prompt_cache_key**。

```typescript
// IEndpointBody 新增字段
prompt_cache_key?: string;

// responsesApi.ts — 仅在实验开启时设置
if (experimentService.getConfig(ConfigKey.ResponsesApiPromptCacheKeyEnabled)) {
    body.prompt_cache_key = `${conversationId}:${endpoint.family}`;
}
```

**功能说明**：

- 用于 Responses API 的 prompt 缓存
- key 格式：`${conversationId}:${modelFamily}`（如 `uuid:gpt-5.1`）
- 需要服务端实验开关 `ResponsesApiPromptCacheKeyEnabled` 启用
- 属于 Responses API 的性能优化特性

**其他变化**：

- Responses API `effort` 恢复默认值 `'medium'`（v0.41.x 无默认值）
- Messages API `effort` 增加 type 检查：`thinkingConfig?.type === 'adaptive'` 才发送 effort

**对 OCCO 的影响**：

- OCCO 未实现 prompt 缓存，暂不需要此字段
- Header 无变化
- 如需实现：需为每个会话维护 conversationId，并在 Responses API body 中添加 prompt_cache_key

### 9.5 升级到 Messages API 的考虑

参见 `docs/claude-messages-api-migration.md`。主要变化：

- 使用 `@anthropic-ai/sdk` 替代 `@ai-sdk/github-copilot`
- thinking 从 `body.thinking_budget` 改为 `thinking: { type: 'adaptive' }`
- 需要额外的 `anthropic-beta` 头
- 需要 `anthropic-version` 头
- 路由到 `/v1/messages` 端点

### 9.6 版本号选择原则

- 版本号必须是服务端已知的有效版本，否则 466
- 建议选择当前实际在分发的稳定版本（marketplace 可查）
- 版本号影响服务端的功能门控（feature gating）

### 9.7 版本号修改位置

修改版本时需要更新 `index.mjs` 中的 **5 处**：

| 位置             | 行号（约） | 内容                                            |
| ---------------- | ---------- | ----------------------------------------------- |
| HEADERS 常量     | 15         | `"User-Agent": "GitHubCopilotChat/X.Y.Z"`       |
| HEADERS 常量     | 16         | `"Editor-Version": "vscode/A.B.C"`              |
| HEADERS 常量     | 17         | `"Editor-Plugin-Version": "copilot-chat/X.Y.Z"` |
| OAuth 设备码流程 | ~519       | `"User-Agent": "GitHubCopilotChat/X.Y.Z"`       |
| Token 轮询流程   | ~552       | `"User-Agent": "GitHubCopilotChat/X.Y.Z"`       |

> 注意：前3处在 HEADERS 常量中，后2处在 OAuth 认证流程中。User-Agent 和 Editor-Plugin-Version 的版本号必须一致。

### 9.8 版本号与 VS Code 引擎版本对应关系

| Copilot Chat 版本 | VS Code 引擎要求 | 推荐 Editor-Version |
| ----------------- | ---------------- | ------------------- |
| v0.36.x           | ^1.109.0         | vscode/1.109.1      |
| v0.37.x           | ^1.109.0         | vscode/1.109.1      |
| v0.38.x           | ^1.110.0         | vscode/1.110.1      |
| v0.39.x           | ^1.111.0         | vscode/1.111.x      |
| v0.40.x           | ^1.111.0         | vscode/1.111.x      |
| v0.41.x           | ^1.111.0         | vscode/1.111.x      |
| v0.42.x           | ^1.111.0         | vscode/1.111.x      |

### 9.9 OCCO 版本变迁历史

| Commit  | 模拟版本 | Intent                | 说明                                  |
| ------- | -------- | --------------------- | ------------------------------------- |
| 8349a34 | 0.35.0   | conversation-edits    | 最早期版本                            |
| d116907 | 0.38.2   | conversation-panel    | 改用 Panel 意图                       |
| 2dffcb2 | 0.39.x   | conversation-subagent | 尝试升级，计费异常                    |
| f25621e | 0.38.2   | conversation-panel    | 回退                                  |
| d93a37d | 0.37.9   | conversation-agent    | 对齐 v0.37.9 行为                     |
| 329fb48 | 0.38.2   | conversation-agent    | 修复 466 错误                         |
| 93b0963 | 0.38.2   | conversation-agent    | 添加 X-Agent-Task-Id                  |
| 0bd0e5d | 0.38.2   | conversation-agent    | 添加 X-Interaction-Id，修复变体默认值 |
