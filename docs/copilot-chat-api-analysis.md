# VS Code Copilot Chat 官方插件 API 请求行为分析

> 分析日期：2026-03-27 (updates: 2026-04-13)
> 对比版本：OCCO 模拟 v0.38.2 vs 官方 v0.36.0 ~ v0.44.x (main)
> 源码仓库：microsoft/vscode-copilot-chat（MIT，2025年12月开源）

## 一、版本演进时间线

| 版本      | VS Code 引擎 | 关键变化                                                                                                                                                                                                                                                                                                                                      |
| --------- | ------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| v0.36.0   | ^1.108.0     | 最早可获取的公开版本。单一 HTTP 路径，X-Initiator 在 chatMLFetcher.ts，X-Interaction-Type 无条件设置，thinking 仅 enabled/disabled                                                                                                                                                                                                            |
| v0.37.0   | ^1.109.0     | 同 v0.36.0，增加 context_management 字段                                                                                                                                                                                                                                                                                                      |
| v0.37.8/9 | ^1.109.0     | 同上，最后一个单一 HTTP 路径版本                                                                                                                                                                                                                                                                                                              |
| v0.38.0   | ^1.110.0     | **重大重构**：拆分 HTTP/WebSocket 双路径，X-Interaction-Type 改为有条件，新增 X-Agent-Task-Id，thinking 增加 adaptive 类型，新增 output_config.effort，新增 conversation-subagent/conversation-background                                                                                                                                     |
| v0.38.2   | ^1.110.0     | 同 v0.38.0 结构                                                                                                                                                                                                                                                                                                                               |
| v0.39.0   | ^1.111.0     | X-Interaction-Type 回归无条件（agentInteractionType 逻辑末尾返回 intent 而非 undefined），X-Agent-Task-Id 也变为无条件                                                                                                                                                                                                                        |
| v0.40.0   | ^1.111.0     | capabilities 动态化（supportsAdaptiveThinking 等从 modelMetadata 获取），Gemini function calling mode 实验                                                                                                                                                                                                                                    |
| v0.41.0   | ^1.111.0     | Messages API 思维控制从 `!disableThinking` 改为 `enableThinking`（正向控制），新增 `forceExtendedThinking` 实验，Responses API effort 来源从实验配置改为 `options.reasoningEffort`                                                                                                                                                            |
| v0.41.1/2 | ^1.111.0     | 与 v0.41.0 所有关键文件（networking.ts, chatEndpoint.ts, messagesApi.ts, responsesApi.ts）**完全一致**                                                                                                                                                                                                                                        |
| v0.42.x   | ^1.111.0     | IEndpointBody 新增 `prompt_cache_key` 字段，Responses API 支持 prompt 缓存（`ResponsesApiPromptCacheKeyEnabled` 实验），effort 默认值改为 `'medium'`                                                                                                                                                                                          |
| v0.43.0   | ^1.115.0     | **`forceExtendedThinking` 全面移除**（#4966）。WebSocket 改为按会话复用连接（#4827，去掉 turnId 参数）。内联摘要（inline summarization, #4956）。effort guard 改为 `supportsReasoningEffort?.length`。`AnthropicPromptOptimization` 移除，Claude 4.6 优化提示成为默认（#4941）。OTel 增强。Chat replay 移除（#4879）。Anthropic SDK 0.81→0.82 |
| v0.44.0   | ^1.115.0     | 仅 1 commit（#4916）：Subagent 遥测增强（`parentToolCallId`、`requestId`、`parentChatSessionId`、`debugLogLabel` 传播至 OTel），`headerRequestId` fallback 逻辑，autopilot retry 消息区分模式                                                                                                                                                 |
| main      | ^1.115.0     | 即 v0.44.0。chatEndpoint.ts 的 thinking 逻辑已从 `customizeCapiBody()` 移出（仅剩 Gemini function calling mode），思维预算逻辑迁移至 anthropicProvider.ts（BYOK 路径）。X-Initiator 确认仍存在于 chatMLFetcher.ts line ~1340                                                                                                                  |

> **X-Initiator 全版本确认**：通过 grep.app 搜索 main 分支确认，`X-Initiator` 从 v0.36.0 到 main 一直存在于 chatMLFetcher.ts 的 additionalHeaders 中（line ~1340），**从未被移除**。networking.ts HTTP 路径从未有过 X-Initiator。

## 二、HTTP Headers 详细分析

### 2.1 官方请求头架构

官方插件的 header 分布在两个层级：

**networking.ts — `networkRequest()` 核心头：**

```typescript
const headers: ReqHeaders = {
  Authorization: `Bearer ${secretKey}`, // Copilot session token
  "X-Request-Id": requestId, // UUID
  "OpenAI-Intent": intent, // 动态，来自 locationToIntent()
  "X-GitHub-Api-Version": "2025-05-01",
  ...additionalHeaders, // 来自 chatMLFetcher.ts
  ...(endpoint.getExtraHeaders ? endpoint.getExtraHeaders(location) : {}),
};
// v0.38.0+ 新增（有条件或无条件，取决于版本）：
// agentInteractionType 决定值：subagent → conversation-subagent，background → conversation-background，其他 → intent
headers["X-Interaction-Type"] = agentInteractionType; // v0.37.x 直接用 intent; v0.38.x 有条件; v0.39.x+ 无条件
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
  !this.options.request.subAgentInvocationId &&
  !this.options.request.isSystemInitiated) ||
  this.stopHookUserInitiated;
```

含义：首次迭代 + 非续接 + 非子Agent调用 + 非系统发起 → user；工具调用后续轮次 → agent。`stopHookUserInitiated` 可覆盖（stop hook 触发时强制 user）

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

同样，`searchSubagentToolCallingLoop.ts` 直接硬编码 `userInitiatedRequest: false` 和 `requestKindOptions: { kind: 'subagent' }`。`executionSubagentToolCallingLoop.ts` 仅硬编码 `userInitiatedRequest: false`，**不传** `requestKindOptions`（依赖上层 defaultIntentRequestHandler 的传递）。

**完整的 intent 分配矩阵：**

| 场景                       | requestKindOptions       | X-Interaction-Type        | X-Initiator |
| -------------------------- | ------------------------ | ------------------------- | ----------- |
| 普通对话（首次）           | `undefined`              | `conversation-agent`¹     | `user`      |
| 普通对话（tool follow-up） | `undefined`              | `conversation-agent`¹     | `agent`     |
| Subagent（首次）           | `{ kind: 'subagent' }`   | `conversation-subagent`   | `agent`²    |
| Subagent（tool follow-up） | `{ kind: 'subagent' }`   | `conversation-subagent`   | `agent`²    |
| Background（标题生成等）   | `{ kind: 'background' }` | `conversation-background` | —           |

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

**v0.36.0 ~ v0.42.x 的逻辑（main 已移除，见下方注释）：**

```typescript
// chatEndpoint.ts → customizeCapiBody()
// ⚠️ 注意：此代码块在 main (v0.44.0) 上已从 customizeCapiBody() 中移除，
// 仅剩 Gemini function calling mode。思维预算逻辑迁移至 anthropicProvider.ts（BYOK 路径）。
// 以下为 v0.36.0 ~ v0.42.x 历史版本的逻辑：
if (
  isAnthropicFamily(this) &&
  !options.disableThinking &&
  isConversationAgent
) {
  const thinkingBudget = this._getThinkingBudget();
  if (thinkingBudget) body.thinking_budget = thinkingBudget;
}
```

**`_getThinkingBudget()` 计算方式**（v0.36.0 ~ v0.42.x 在 chatEndpoint.ts；main 迁移至 anthropicProvider.ts）：

```typescript
const configuredBudget = getExperimentConfig(ConfigKey.AnthropicThinkingBudget);
// 默认值：16000
const normalizedBudget =
  configuredBudget > 0
    ? Math.max(1024, configuredBudget) // 最小 1024（main 用三元运算符实现，逻辑等价）
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

| 版本    | 启用条件                | adaptive 支持 | effort 控制                                     | forceExtendedThinking                         |
| ------- | ----------------------- | ------------- | ----------------------------------------------- | --------------------------------------------- |
| v0.37.x | `!disableThinking`      | ❌            | ❌                                              | ❌                                            |
| v0.38.x | `!disableThinking`      | ✅            | ✅ (adaptive 模型)                              | ❌                                            |
| v0.40.0 | `!disableThinking`      | ✅ (动态化)   | ✅ (adaptive 模型)                              | ❌                                            |
| v0.41.x | `enableThinking` (正向) | ✅            | ✅ (adaptive 模型, 带验证)                      | ✅ (实验性, `AnthropicForceExtendedThinking`) |
| v0.42.x | `enableThinking`        | ✅            | ✅ (仅 adaptive + type match)                   | ✅ (同 v0.41.x)                               |
| v0.43.x | `enableThinking`        | ✅            | ✅ (gated by `supportsReasoningEffort?.length`) | ❌ (**已移除** #4966)                         |
| v0.44.x | `enableThinking`        | ✅            | ✅ (同 v0.43.x)                                 | ❌ (已移除)                                   |

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

**v0.43.x 变化（forceExtendedThinking 移除）：**

```typescript
// forceExtendedThinking guard 移除，adaptive 条件简化为：
if (!thinkingExplicitlyDisabled) {  // 原为 !thinkingExplicitlyDisabled && !forceExtendedThinking
    if (endpoint.supportsAdaptiveThinking) {
        thinkingConfig = { type: 'adaptive' };
    } else if (...) {
        thinkingConfig = { type: 'enabled', budget_tokens: computed };
    }
}

// effort guard 改为检查 supportsReasoningEffort 数组长度：
const candidateEffort = endpoint.supportsReasoningEffort?.length
    ? (configService.getConfig(ConfigKey.TeamInternal.AnthropicThinkingEffort) ?? reasoningEffort)
    : undefined;
// 注：TeamInternal.AnthropicThinkingEffort 优先于 reasoningEffort（用于 evals）

// CacheBreakpoint 修复：空白文本块改为 pendingCacheControl 延迟模式
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

| 版本    | effort 来源                                                                                            | 默认值                                          |
| ------- | ------------------------------------------------------------------------------------------------------ | ----------------------------------------------- |
| v0.37.x | `configService.getExperimentBasedConfig(ResponsesApiReasoningEffort)`                                  | 'medium' (当 config='default' 时)               |
| v0.38.x | 同上                                                                                                   | 同上                                            |
| v0.39.x | 同上                                                                                                   | 同上                                            |
| v0.40.0 | 同上                                                                                                   | 同上                                            |
| v0.41.x | `options.reasoningEffort`（从调用方传入）                                                              | 无默认                                          |
| v0.42.x | `options.reasoningEffort \|\| 'medium'`                                                                | 'medium'                                        |
| v0.43.x | `effortFromSetting \|\| options.reasoningEffort \|\| 'medium'`，但需 `supportsReasoningEffort?.length` | 'medium'（仅当 supportsReasoningEffort 存在时） |
| v0.44.x | 同 v0.43.x                                                                                             | 同上                                            |

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

| 项目                                | 官方行为                                                                                            | OCCO 行为                          | 影响                              |
| ----------------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------- | --------------------------------- |
| X-Initiator 判定逻辑                | 基于状态机：`iterationNumber===0 && !isContinuation && !subAgentInvocationId && !isSystemInitiated` | 基于 body 结构分析 + parentID 覆盖 | **中**，见下方 §7.3 详细分析      |
| X-Interaction-Type subagent 分支    | subagent→`conversation-subagent`，background→`conversation-background`                              | 始终 `conversation-agent`          | 低，无证据影响计费（仅遥测/路由） |
| OpenAI-Intent 动态性                | locationToIntent() 动态                                                                             | 固定 conversation-agent            | 低，opencode 本质是 Agent         |
| GPT 默认 reasoning effort           | 始终发送 medium                                                                                     | 仅变体选择时发送                   | 中，可能影响默认推理质量          |
| body.prompt_cache_key               | `${conversationId}:${endpoint.family}`（实验门控）                                                  | 缺少                               | **中**，影响缓存命中率→间接成本   |
| body.text.verbosity                 | 模型级别配置（gpt-5.1→`'low'`）                                                                     | 缺少                               | 低，间接影响输出长度              |
| body.truncation                     | `'auto'` 或 `'disabled'`                                                                            | 缺少                               | 低，间接影响 token 使用           |
| body.store                          | `false`                                                                                             | 缺少                               | 低，仅影响数据保留策略            |
| X-VSCode-User-Agent-Library-Version | 存在                                                                                                | 缺少                               | 低，服务端不依赖                  |
| Messages API                        | 支持 /v1/messages                                                                                   | 不支持                             | 低，ChatCompletions 功能等价      |
| WebSocket initiator 位置            | WS 路径：payload body 中 `initiator: 'user'\|'agent'`                                               | HTTP only（header X-Initiator）    | 无，OCCO 仅使用 HTTP              |

### 7.3 X-Initiator 判定逻辑差异详细分析

**官方完整判定链** (toolCallingLoop.ts → chatMLFetcher.ts → networking.ts)：

```typescript
// toolCallingLoop.ts L1469-1474
const userInitiatedRequest =
  (iterationNumber === 0
    && !isContinuation
    && !this.options.request.subAgentInvocationId
    && !this.options.request.isSystemInitiated)
  || this.stopHookUserInitiated;

// chatMLFetcher.ts L1340-1346 — 传入 additionalHeaders
'X-Initiator': userInitiatedRequest ? 'user' : 'agent'

// networking.ts L382-389 — 不做二次判断，直接合并
{ ...additionalHeaders }
```

**OCCO 判定逻辑** (index.mjs isAgent 检测)：

```javascript
// ChatCompletions 路径 (body.messages + URL includes 'completions')
isAgent = last?.role !== "user" || imgMsg(last) || userCount !== 1

// Responses API 路径 (body.input)
isAgent = last?.role !== "user" || imgMsg(last) || userCount !== 1

// Messages API 路径 (body.messages, 非 completions URL)
isAgent = !(last?.role === "user"
  && last.content.some(p => p?.type !== "tool_result"))
  || imgMsg(last) || userCount !== 1

// parentID 覆盖（来自 chat.headers hook）
finalIsAgent = bodyIsAgent || initHeaders["x-initiator"] === "agent"

// 最终 header
"X-Initiator": isAgent ? "agent" : "user"
```

**关键差异场景**：

| 场景                               | 官方结果 | OCCO 结果            | 说明                                                      |
| ---------------------------------- | -------- | -------------------- | --------------------------------------------------------- |
| 首次用户消息                       | ✅ user  | ✅ user              | 一致                                                      |
| 工具调用迭代 (iterationNumber > 0) | agent    | body分析决定         | 若最后消息是 tool result 后 user 追问，OCCO 可能返回 user |
| 继续请求 (isContinuation)          | agent    | body分析决定         | OCCO 无 isContinuation 概念，看 body 结构                 |
| 子代理请求 (subAgentInvocationId)  | agent    | agent (parentID覆盖) | OCCO 用 parentID 机制达到同等效果                         |
| isSystemInitiated 请求             | agent    | body分析决定         | OCCO 无此概念                                             |
| stopHook 恢复                      | user     | body分析决定         | OCCO 无 stopHook 机制                                     |

> **风险评估**：OCCO 的 body 分析在大多数场景下与官方一致（工具调用后 body 结构自然呈现 agent 特征），但在 continuation 和 system-initiated 边界情况下可能产生分歧。`parentID` 覆盖机制在子代理场景下有效保障了一致性。

### 7.4 requestKindOptions 死代码

官方 networking.ts 的 `agentInteractionType` 分支中，`{kind: 'background'}` 路径（→ `conversation-background`）在当前代码中**从未被调用**。接口已定义但无生产代码设置该值。仅 `{kind: 'subagent'}` 有两处使用：

- `defaultIntentRequestHandler.ts`：条件设置（当 `subAgentInvocationId` 存在时）
- `searchSubagentToolCallingLoop.ts`：硬编码
- `executionSubagentToolCallingLoop.ts`：**不设置** requestKindOptions（其 X-Interaction-Type 退化为 `intent` 值）

> OCCO 始终使用 `conversation-agent` 作为 X-Interaction-Type，这与 executionSubagent 的实际行为一致（都使用非 subagent 的 intent 值）。

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

### 8.3 prompt_cache_key 缺失的成本影响

官方 v0.42.x+ 在 Responses API 请求中发送 `prompt_cache_key`（格式 `${conversationId}:${endpoint.family}`），用于服务端 prompt 缓存。

- **缓存命中**时，重复的 system prompt 和历史消息作为 cached input tokens 计费（价格低于 uncached）
- **缺少此字段**时，每次请求都按完整 input token 计费
- 此字段本身不产生额外费用，但能显著降低长会话的 token 成本
- 实验门控：`ConfigKey.ResponsesApiPromptCacheKeyEnabled`——服务端可能尚未全量开放

**影响评估**：**中等**。对短会话（1-2轮）影响极小；对长会话（10+ 轮）可能节省 30-50% 的 input token 成本。OCCO 当前不维护 conversationId 语义（每次请求独立），实现需要：

1. 为每个会话维护稳定的 conversationId
2. 在 Responses API body 中添加 `prompt_cache_key: "${conversationId}:${modelFamily}"`

### 8.4 Responses API isAgent 检测边界情况

OCCO 的 Responses API isAgent 检测（`body.input` 路径）存在一个边界情况：

```
场景：工具调用后，body.input 结构为 [...tool_outputs, {role: "user", content: "继续"}]
```

- **官方**：`iterationNumber > 0` → 始终 `agent`（无论 body 结构）
- **OCCO**：`last.role === "user" && userCount === 1` → **`user`**

此差异可能导致 OCCO 在工具调用迭代中将本应计为 agent 的请求标记为 user。

**影响评估**：**低到中**。取决于服务端是否基于 X-Initiator 区分 premium 计费。从 v0.40.0 计费事件看，X-Initiator 对计费有影响，但 OCCO 的 body 分析在绝大多数多轮场景下自然返回 agent（因为 userCount > 1）。唯一风险是首次工具调用结果后仅附加一条 user 消息的极端情况。

> 注意：`function_call_output` 类型的 item 在 Responses API 路径中**未做特殊处理**，不影响 role 计数。`previous_response_id` 也不参与检测。

### 8.5 body 字段对计费的直接/间接影响总结

| 字段               | 计费影响       | 说明                                           |
| ------------------ | -------------- | ---------------------------------------------- |
| `prompt_cache_key` | **间接**（中） | 缺失→无缓存→更高 input token 成本              |
| `text.verbosity`   | **间接**（低） | `'low'` 减少输出量→节省 output token，非硬限制 |
| `truncation`       | **间接**（低） | `'auto'` 自动截断长上下文→节省 input token     |
| `store`            | **无**         | 仅控制数据保留策略，不影响计费/配额            |
| `reasoning.effort` | **间接**（中） | 影响推理 token 消耗量，OCCO 仅在变体选择时发送 |

> **Copilot 配额核心机制**：由 `modelMetadata.billing`（`is_premium`, `multiplier`, `restricted_to`）从 endpointProvider 返回，不由上述 body 字段驱动。这些字段影响的是实际 token 消耗量，而非配额计算公式。

### 8.6 Intent 变迁历史

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
- ⚠️ 注意：v0.43.0+ 中 `customizeCapiBody()` 的 thinking 逻辑已移除（仅剩 Gemini function calling mode），迁移至 anthropicProvider.ts（BYOK 路径）

**对 OCCO 的影响**：

- OCCO 的模型定义是硬编码的，不受服务端动态化影响
- ChatCompletions 路径的 thinking_budget 行为所有版本一致
- 如需支持 Messages API，需要实现 adaptive thinking 逻辑

### 9.3 升级到 v0.41.x 的额外考虑

v0.41.x 的关键变化集中在思维链控制和推理参数来源：

**Messages API 变化（OCCO 未使用，备记录）：**

- `enableThinking`（正向）替代 `!disableThinking`（双重否定）
- 新增 `forceExtendedThinking` 实验：强制使用 extended thinking 而非 adaptive（⚠️ v0.43.0 中已全面移除 #4966）
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

### 9.5 升级到 v0.43.x 的额外考虑

v0.43.0 是自 v0.38.0 以来最大的一次变更（~120 commits, 266 files changed），关键变化：

**1. `forceExtendedThinking` 全面移除（#4966）：**

- `AnthropicForceExtendedThinking` 配置项及所有引用删除
- messagesApi.ts: adaptive gate 从 `!thinkingExplicitlyDisabled && !forceExtendedThinking` 简化为 `!thinkingExplicitlyDisabled`
- chatEndpoint.ts: beta header 条件从 `!this.supportsAdaptiveThinking || forceExtendedThinking` 简化为 `!this.supportsAdaptiveThinking`
- anthropicProvider.ts (BYOK): `forceExtendedThinking` 移除，`supportsAdaptiveThinking` 不再被 forceNonAdaptive 门控

**2. Claude 4.6 优化提示成为默认（#4941）：**

- `AnthropicPromptOptimization` 配置项移除，优化后的提示直接作为默认值

**3. WebSocket 改为按会话复用连接（#4827）：**

```typescript
// v0.42.x: 按 (conversationId, turnId) 管理
getOrCreateConnection(conversationId, turnId, headers)
hasActiveConnection(conversationId, turnId)
closeConnection(conversationId, turnId?)

// v0.43.x: 按 conversationId 管理，turnId 移到 sendRequest()
getOrCreateConnection(conversationId, headers)
hasActiveConnection(conversationId)
closeConnection(conversationId)
connection.sendRequest(request, { userInitiated, turnId }, token)
// 连接跟踪 _turnId, _previousTurnId, _hadActiveRequest（遥测用）
```

**4. 内联摘要（Inline Summarization, #4956）：**

- 新增 `InlineSummarizationRequestedMetadata` 系统
- `_isInlineSummarizationRequest` 标志
- 摘要提取/存储，"Compacting conversation..." 进度提示
- 摘要期间抑制流式输出，`applySummaryToRound()` 方法

**5. effort guard 统一为数组长度检查：**

```typescript
// v0.42.x: boolean 检查
endpoint.supportsAdaptiveThinking && thinkingConfig?.type === "adaptive";

// v0.43.x: 数组长度检查（Messages API 和 Responses API 统一）
endpoint.supportsReasoningEffort?.length;
// anthropicProvider.ts (BYOK): 也改为 includes 检查
modelCapabilities?.supportsReasoningEffort; // 数组类型
// effort 类型扩展为 'low' | 'medium' | 'high' | 'max'
```

**6. 新增 Team-Internal 配置（用于 evals）：**

- `ConfigKey.TeamInternal.ResponsesApiReasoningEffort`：Responses API effort 覆盖
- `ConfigKey.TeamInternal.AnthropicThinkingEffort`：Messages API effort 覆盖
- 这些配置优先于 `options.reasoningEffort`

**7. 其他变化：**

- OTel 增强：workspace metadata, custom mode name, `turn_start`/`turn_end` events, `tools_available` event
- `AgentHistorySummarizationWithPromptCache` → `AgentHistorySummarizationInline`（重命名）
- Chat replay 功能移除（#4879）
- AGENTS.md/CLAUDE.md 发现修复（#4989）
- Anthropic SDK 0.81.0→0.82.0
- `create_file` 改为单批次运行（#4920）
- 新 DI：`@IGitService`（toolCallingLoop.ts）
- automodeService.ts 重构为 `FetchedValue<AutoModeAPIResponse>` 模式
- chatModelCapabilities.ts 新增 `isHiddenModelB()`、`isVSCModelC()`/`isVSCModelD()`

**对 OCCO 的影响**：

- ChatCompletions 路径（Claude）：chatEndpoint.ts thinking 逻辑已移除，但 OCCO 使用自己的实现，不受影响
- WebSocket：如计划实现 WS 路径，需注意连接复用改为按 conversationId（不再含 turnId）
- Header 无变化
- 需更新版本号和 Engine-Version 至 `vscode/1.115.x`

### 9.6 升级到 v0.44.x 的额外考虑

v0.44.0 仅含 1 commit（#4916 "Yemohyle/subagent telem"），变化极小：

**Subagent 遥测增强：**

- `parentToolCallId` 添加到 execution/search subagent loop 选项和遥测属性
- `requestId`（= subAgentInvocationId）添加到 subagent 遥测
- `parentChatSessionId` 和 `debugLogLabel` 通过 CapturingToken 传播到 OTel attributes（`PARENT_CHAT_SESSION_ID`, `DEBUG_LOG_LABEL`）
- `IChatDebugFileLoggerService.startChildSession()` 用于子代理调试日志

**chatMLFetcher.ts：**

- `headerRequestId` 处理：当服务端未回传 x-request-id 时保留 `ourRequestId` 作为 fallback
- `headerRequestId` 从手动属性设置移到 `baseTelemetry` 构造

**toolCallingLoop.ts：**

- autopilot retry/continue 消息区分 autopilot 模式和非 autopilot 模式

**对 OCCO 的影响**：无。纯遥测增强，不影响 API 请求行为。

### 9.7 升级到 Messages API 的考虑

参见 `docs/claude-messages-api-migration.md`。主要变化：

- 使用 `@anthropic-ai/sdk` 替代 `@ai-sdk/github-copilot`
- thinking 从 `body.thinking_budget` 改为 `thinking: { type: 'adaptive' }`
- 需要额外的 `anthropic-beta` 头
- 需要 `anthropic-version` 头
- 路由到 `/v1/messages` 端点

### 9.8 版本号选择原则

- 版本号必须是服务端已知的有效版本，否则 466
- 建议选择当前实际在分发的稳定版本（marketplace 可查）
- 版本号影响服务端的功能门控（feature gating）

### 9.9 版本号修改位置

修改版本时需要更新 `index.mjs` 中的 **5 处**：

| 位置             | 行号（约） | 内容                                            |
| ---------------- | ---------- | ----------------------------------------------- |
| HEADERS 常量     | 15         | `"User-Agent": "GitHubCopilotChat/X.Y.Z"`       |
| HEADERS 常量     | 16         | `"Editor-Version": "vscode/A.B.C"`              |
| HEADERS 常量     | 17         | `"Editor-Plugin-Version": "copilot-chat/X.Y.Z"` |
| OAuth 设备码流程 | ~519       | `"User-Agent": "GitHubCopilotChat/X.Y.Z"`       |
| Token 轮询流程   | ~552       | `"User-Agent": "GitHubCopilotChat/X.Y.Z"`       |

> 注意：前3处在 HEADERS 常量中，后2处在 OAuth 认证流程中。User-Agent 和 Editor-Plugin-Version 的版本号必须一致。

### 9.10 版本号与 VS Code 引擎版本对应关系

| Copilot Chat 版本 | VS Code 引擎要求 | 推荐 Editor-Version |
| ----------------- | ---------------- | ------------------- |
| v0.36.x           | ^1.108.0         | vscode/1.108.x      |
| v0.37.x           | ^1.109.0         | vscode/1.109.1      |
| v0.38.x           | ^1.110.0         | vscode/1.110.1      |
| v0.39.x           | ^1.111.0         | vscode/1.111.x      |
| v0.40.x           | ^1.111.0         | vscode/1.111.x      |
| v0.41.x           | ^1.111.0         | vscode/1.111.x      |
| v0.42.x           | ^1.111.0         | vscode/1.111.x      |
| v0.43.x           | ^1.115.0         | vscode/1.115.x      |
| v0.44.x (main)    | ^1.115.0         | vscode/1.115.x      |

### 9.11 OCCO 版本变迁历史

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
