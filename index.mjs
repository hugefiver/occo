/**
 * @type {import('@opencode-ai/plugin').Plugin}
 */
export async function OccoAuthPlugin({ client }) {
  const CLIENT_ID = "Iv1.b507a08c87ecfe98";
  const DEFAULT_API_URL = "https://api.githubcopilot.com";
  // Default: follow Copilot Chat behavior and use token endpoints.api.
  // Set OCCO_USE_TOKEN_ENDPOINT_API=0 to force legacy DEFAULT_API_URL.
  const USE_TOKEN_ENDPOINT_API =
    process.env.OCCO_USE_TOKEN_ENDPOINT_API !== "0";

  const TOKEN_URL = "https://api.github.com/copilot_internal/v2/token";
  const POLLING_MARGIN_MS = 3000;
  const HEADERS = {
    "User-Agent": "GitHubCopilotChat/0.38.2",
    "Editor-Version": "vscode/1.110.1",
    "Editor-Plugin-Version": "copilot-chat/0.38.2",
    "Copilot-Integration-Id": "vscode-chat",
    "X-GitHub-Api-Version": "2025-05-01",
  };

  // Resolved dynamically from token response's endpoints.api (unless legacy mode)
  let resolvedApiUrl = DEFAULT_API_URL;

  // Fallback: models known to support /v1/messages (used when /models API fails)
  const MESSAGES_API_MODELS = new Set([
    "claude-opus-4.6",
    "claude-opus-4.5",
    "claude-sonnet-4.6",
    "claude-sonnet-4.5",
    "claude-sonnet-4",
    "claude-haiku-4.5",
  ]);

  // Cached /models API response (refreshed each auth.loader call)
  let cachedRemoteMap = {};

  // Synthetic image-attachment messages (from tool results) should not count
  // as real user turns for quota purposes.
  const SYNTHETIC_ATTACHMENT_PROMPT = "Attached image(s) from tool result:";

  function imgMsg(msg) {
    if (msg?.role !== "user") return false;
    const content = msg.content;
    if (typeof content === "string")
      return content === SYNTHETIC_ATTACHMENT_PROMPT;
    if (!Array.isArray(content)) return false;
    return content.some(
      (part) =>
        (part?.type === "text" || part?.type === "input_text") &&
        part.text === SYNTHETIC_ATTACHMENT_PROMPT,
    );
  }

  // Cache session → interaction UUID mapping
  const interactionIds = new Map();

  // ---------------------------------------------------------------------------
  // Token refresh helper — reused at loader init and per-request
  // ---------------------------------------------------------------------------
  async function refreshTokenIfNeeded(getAuth) {
    const info = await getAuth();
    if (!info || info.type !== "oauth") return info;
    if (!info.refresh) return info;

    // Refresh if: no access token, or expired/about-to-expire
    // (stored expires already has a 5-minute buffer baked in)
    const needsRefresh = !info.access || info.expires < Date.now();
    if (!needsRefresh) return info;

    const response = await fetch(TOKEN_URL, {
      headers: {
        Accept: "application/json",
        Authorization: `Bearer ${info.refresh}`,
        ...HEADERS,
      },
    });
    if (!response.ok) {
      throw new Error(`Token refresh failed: ${response.status}`);
    }
    const tokenData = await response.json();

    // Dynamically resolve API URL from token response (optional)
    if (USE_TOKEN_ENDPOINT_API && tokenData.endpoints?.api) {
      resolvedApiUrl = tokenData.endpoints.api;
    }

    await client.auth.set({
      path: { id: "occo" },
      body: {
        type: "oauth",
        refresh: info.refresh,
        access: tokenData.token,
        expires: tokenData.expires_at * 1000 - 5 * 60 * 1000,
      },
    });
    info.access = tokenData.token;
    info.expires = tokenData.expires_at * 1000 - 5 * 60 * 1000;
    return info;
  }

  // ---------------------------------------------------------------------------
  // Variant definitions
  // ---------------------------------------------------------------------------

  // Claude variants: thinking_budget via ChatCompletions path.
  // Cap: Math.min(32000, maxOutputTokens - 1, normalizedBudget).
  // Opus 4.5+: thinking enabled by default via model options, thinking/max variants available.
  // Lower tiers (sonnet/haiku): thinking opt-in only via variant, no max.
  //
  // SDK auto-generates low/medium/high reasoningEffort variants for Claude
  // models on @ai-sdk/github-copilot. Disable them so only our custom
  // thinking_budget variants are exposed.
  const DISABLED_SDK_VARIANTS = {
    low: { disabled: true },
    medium: { disabled: true },
    high: { disabled: true },
  };
  const CLAUDE_OPUS_VARIANTS = {
    ...DISABLED_SDK_VARIANTS,
    thinking: { thinking_budget: 16000 },
    max: { thinking_budget: 32000 },
  };
  const CLAUDE_LOWER_VARIANTS = {
    ...DISABLED_SDK_VARIANTS,
    thinking: { thinking_budget: 16000 },
  };
  const CLAUDE_SONNET4_VARIANTS = {
    ...DISABLED_SDK_VARIANTS,
    thinking: { thinking_budget: 15999 },
  };

  // GPT reasoning effort variants (for mini models): default=high (via SDK)
  const GPT_REASONING_VARIANTS = {
    low: {
      reasoningEffort: "low",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
    medium: {
      reasoningEffort: "medium",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
    high: {
      reasoningEffort: "high",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
  };

  // GPT reasoning effort variants with xhigh (for gpt-5.1, gpt-5.2 non-codex): default=high
  const GPT_REASONING_VARIANTS_XHIGH = {
    ...GPT_REASONING_VARIANTS,
    xhigh: {
      reasoningEffort: "xhigh",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
  };

  // GPT Codex variants: only high and xhigh
  const GPT_CODEX_VARIANTS = {
    high: {
      reasoningEffort: "high",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
    xhigh: {
      reasoningEffort: "xhigh",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
  };

  // ---------------------------------------------------------------------------
  // Hardcoded model definitions from Copilot API.
  // Context = min(max_context_window_tokens, max_output_tokens + max_prompt_tokens).
  //
  // Variant assignment:
  //   claude-opus-4.5+ → thinking(16000) / max(32000)
  //   claude-sonnet/haiku → thinking only (no max)
  //   gemini         → no variants
  //   gpt-5.1, gpt-5.2, gpt-5.4 (non-codex) → default(high) / low/medium/high/xhigh
  //   gpt-5.*-codex/codex-max → default(high) / high/xhigh only
  //   gpt-5-mini, gpt-5.1-codex-mini → no variants (mini = no reasoning)
  //   gpt-4o/4.1    → no variants (no reasoning)
  //   grok           → no variants
  //   oswe/raptor    → no variants (mini = no reasoning)
  //
  const MODELS = {
    // --- Claude models ---

    // removed from Copilot Pro+ plan
    // "claude-opus-4.6-fast": {
    //   name: "Claude Opus 4.6 (fast mode)",
    //   reasoning: true,
    //   tool_call: true,
    //   temperature: true,
    //   modalities: { input: ["text", "image"], output: ["text"] },
    //   limit: { context: 192000, output: 64000 },
    //   options: { thinking_budget: 16000 },
    //   variants: CLAUDE_OPUS_VARIANTS,
    // },
    "claude-opus-4.6": {
      name: "Claude Opus 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 192000, output: 64000 },
      options: { thinking_budget: 16000 },
      variants: CLAUDE_OPUS_VARIANTS,
    },
    "claude-opus-4.5": {
      name: "Claude Opus 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
      options: { thinking_budget: 16000 },
      variants: CLAUDE_OPUS_VARIANTS,
    },
    "claude-sonnet-4.6": {
      name: "Claude Sonnet 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
      variants: CLAUDE_LOWER_VARIANTS,
    },
    "claude-sonnet-4.5": {
      name: "Claude Sonnet 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
      variants: CLAUDE_LOWER_VARIANTS,
    },
    "claude-sonnet-4": {
      name: "Claude Sonnet 4",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 144000, output: 16000 },
      variants: CLAUDE_SONNET4_VARIANTS,
    },
    "claude-haiku-4.5": {
      name: "Claude Haiku 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
      variants: CLAUDE_LOWER_VARIANTS,
    },
    // --- Gemini models ---
    "gemini-2.5-pro": {
      name: "Gemini 2.5 Pro",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "gemini-3-flash-preview": {
      name: "Gemini 3 Flash (Preview)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "gemini-3-pro-preview": {
      name: "Gemini 3 Pro (Preview)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "gemini-3.1-pro-preview": {
      name: "Gemini 3.1 Pro",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    // --- GPT models ---
    "gpt-4o": {
      name: "GPT-4o",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 68096, output: 4096 },
    },
    "gpt-4.1": {
      name: "GPT-4.1",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 16384 },
    },
    "gpt-5-mini": {
      name: "GPT-5 mini",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 192000, output: 64000 },
    },
    "gpt-5.1": {
      name: "GPT-5.1",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 192000, output: 64000 },
      variants: GPT_REASONING_VARIANTS_XHIGH,
    },
    "gpt-5.1-codex": {
      name: "GPT-5.1-Codex",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 256000, output: 128000 },
      variants: GPT_CODEX_VARIANTS,
    },
    "gpt-5.1-codex-mini": {
      name: "GPT-5.1-Codex-Mini",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 256000, output: 128000 },
    },
    "gpt-5.1-codex-max": {
      name: "GPT-5.1-Codex-Max",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 256000, output: 128000 },
      variants: GPT_CODEX_VARIANTS,
    },
    "gpt-5.2": {
      name: "GPT-5.2",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 192000, output: 64000 },
      variants: GPT_REASONING_VARIANTS_XHIGH,
    },
    "gpt-5.2-codex": {
      name: "GPT-5.2-Codex",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 400000, output: 128000 },
      variants: GPT_CODEX_VARIANTS,
    },
    "gpt-5.3-codex": {
      name: "GPT-5.3-Codex",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 400000, output: 128000 },
      variants: GPT_CODEX_VARIANTS,
    },
    "gpt-5.4": {
      name: "GPT-5.4",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 400000, output: 128000 },
      variants: GPT_REASONING_VARIANTS_XHIGH,
    },
    // --- Other models ---
    "grok-code-fast-1": {
      name: "Grok Code Fast 1",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "oswe-vscode-prime": {
      name: "Raptor mini (Preview)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 264000, output: 64000 },
    },
  };

  return {
    // -------------------------------------------------------------------------
    // Register "occo" as a provider with Copilot model definitions
    // -------------------------------------------------------------------------
    config: (config) => {
      config.provider = config.provider ?? {};
      config.provider.occo = {
        name: "OCCO",
        npm: "@ai-sdk/github-copilot",
        api: DEFAULT_API_URL,
        models: MODELS,
      };
    },

    // -------------------------------------------------------------------------
    // Subagent quota protection: mark subagent sessions with x-initiator header
    // so they don't consume user's Copilot quota
    // -------------------------------------------------------------------------
    "chat.params": async (incoming, output) => {
      if (incoming.model.providerID !== "occo") return;
      // Match github copilot cli, omit maxOutputTokens for gpt models
      if (incoming.model.api.id.includes("gpt")) {
        output.maxOutputTokens = undefined;
      }
    },

    "chat.headers": async (incoming, output) => {
      if (incoming.model.providerID !== "occo") return;
      // Generate stable interaction UUID per session
      if (!interactionIds.has(incoming.sessionID)) {
        interactionIds.set(incoming.sessionID, crypto.randomUUID());
      }
      output.headers["x-interaction-id"] = interactionIds.get(
        incoming.sessionID,
      );
      try {
        const session = await client.session.get({
          path: { id: incoming.sessionID },
        });
        if (session.data?.parentID) {
          output.headers["x-initiator"] = "agent";
          output.headers["openai-intent"] = "conversation-agent";
          output.headers["x-interaction-type"] = "conversation-agent";
        }
      } catch {}
    },

    // -------------------------------------------------------------------------
    // Auth: session token exchange via Copilot internal API
    // -------------------------------------------------------------------------
    auth: {
      provider: "occo",
      loader: async (getAuth, provider) => {
        const info = await getAuth();
        if (!info || info.type !== "oauth") return {};

        // Proactive token refresh on plugin load
        const refreshedInfo = await refreshTokenIfNeeded(getAuth);

        // Fetch remote model capabilities (updates cache; non-fatal)
        if (refreshedInfo?.access) {
          try {
            const modelsResp = await fetch(`${resolvedApiUrl}/models`, {
              headers: {
                Authorization: `Bearer ${refreshedInfo.access}`,
                ...HEADERS,
              },
            });
            if (modelsResp.ok) {
              const data = await modelsResp.json();
              const map = {};
              for (const r of Array.isArray(data) ? data : data?.data || []) {
                if (r?.id) map[r.id] = r;
              }
              cachedRemoteMap = map;
            }
          } catch {}
        }

        // Tag models that support Anthropic Messages API.
        // Priority: options.messagesApi > remote supported_endpoints > fallback list
        if (provider?.models) {
          for (const [id, model] of Object.entries(provider.models)) {
            const remote = cachedRemoteMap[id];
            const supports = remote?.capabilities?.supports;
            const useMessages =
              model.options?.messagesApi ??
              remote?.supported_endpoints?.includes("/v1/messages") ??
              MESSAGES_API_MODELS.has(id);

            if (!useMessages) continue;

            const isAdaptive =
              model.options?.adaptiveThinking ??
              supports?.adaptive_thinking === true;
            const maxBudget = supports?.max_thinking_budget;

            model.api = {
              npm: "@ai-sdk/anthropic",
              url: `${resolvedApiUrl}/v1`,
            };

            // Clean up user override flags + copilot-format from options
            if (!model.options) model.options = {};
            delete model.options.messagesApi;
            delete model.options.adaptiveThinking;
            delete model.options.thinking_budget;

            // Set default thinking config only when user hasn't overridden
            if (isAdaptive) {
              model.options.thinking ??= { type: "adaptive" };
              model.options.effort ??= "high";
            } else if (maxBudget) {
              model.options.thinking ??= {
                type: "enabled",
                budgetTokens: Math.min(16000, maxBudget),
              };
            }

            // Server-side context editing: prune stale tool results and
            // old thinking blocks to keep long conversations within limits.
            // User can override via options.contextManagement or set to false to disable.
            if (model.options.contextManagement === false) {
              delete model.options.contextManagement;
            } else {
              model.options.contextManagement ??= {
                edits: [
                  { type: "clear_thinking_20251015" },
                  { type: "clear_tool_uses_20250919" },
                ],
              };
            }

            // Convert thinking_budget variants to anthropic SDK format;
            // drop disabled variants (SDK-generated low/medium/high no longer needed)
            if (model.variants) {
              const converted = {};
              for (const [name, opts] of Object.entries(model.variants)) {
                if (opts.disabled) continue;
                if (opts.thinking_budget != null) {
                  converted[name] = {
                    thinking: {
                      type: "enabled",
                      budgetTokens: opts.thinking_budget,
                    },
                  };
                } else {
                  converted[name] = opts;
                }
              }
              model.variants = converted;
            }
          }
        }

        // Zero out costs (Copilot is free)
        if (provider?.models) {
          for (const model of Object.values(provider.models)) {
            model.cost = { input: 0, output: 0, cache: { read: 0, write: 0 } };
          }
        }

        return {
          apiKey: "",
          async fetch(input, init) {
            const info = await refreshTokenIfNeeded(getAuth);
            if (!info || info.type !== "oauth") return fetch(input, init);

            // Parse request body once for routing + detection
            let parsedBody = null;
            try {
              parsedBody =
                typeof init.body === "string"
                  ? JSON.parse(init.body)
                  : init.body;
            } catch {}

            // Rewrite request URL based on model source:
            // Route to resolved prod API (all models go through endpoints.api)
            let url = typeof input === "string" ? input : input.url;

            if (
              USE_TOKEN_ENDPOINT_API &&
              url.startsWith(DEFAULT_API_URL) &&
              resolvedApiUrl !== DEFAULT_API_URL
            ) {
              url = url.replace(DEFAULT_API_URL, resolvedApiUrl);
            }

            function detect(msgs, visionType, isMessagesApi) {
              const last = msgs[msgs.length - 1];
              const userCount = msgs.filter(
                (x) =>
                  x?.role === "user" &&
                  !/^[@/]/.test(x?.content) &&
                  (!isMessagesApi ||
                    (Array.isArray(x?.content) &&
                      x?.content?.some((p) => p?.type !== "tool_result"))),
              ).length;
              const notUser = userCount % 10 !== 1;
              const isAgent =
                notUser ||
                /^[@/]/.test(last?.content) ||
                (isMessagesApi
                  ? !(
                      last?.role === "user" &&
                      Array.isArray(last?.content) &&
                      last.content.some((p) => p?.type !== "tool_result")
                    ) || imgMsg(last)
                  : last?.role !== "user" || imgMsg(last));
              const isVision = msgs.some(
                (msg) =>
                  Array.isArray(msg.content) &&
                  msg.content.some(
                    (part) =>
                      part.type === visionType ||
                      (part.type === "tool_result" &&
                        Array.isArray(part.content) &&
                        part.content.some((n) => n?.type === visionType)),
                  ),
              );
              return { isAgent, isVision };
            }

            // Detect agent calls and vision requests
            const { isAgent: bodyIsAgent, isVision } = (() => {
              try {
                if (parsedBody?.messages && url.includes("completions"))
                  return detect(parsedBody.messages, "image_url");
                if (parsedBody?.input)
                  return detect(parsedBody.input, "input_image");
                if (parsedBody?.messages)
                  return detect(parsedBody.messages, "image", true);
              } catch {}
              return { isAgent: true, isVision: false };
            })();
            const initHeaders = init?.headers ?? {};
            const isAgent =
              bodyIsAgent || initHeaders["x-initiator"] === "agent";

            const requestId = crypto.randomUUID();
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
            if (isVision) {
              headers["Copilot-Vision-Request"] = "true";
            }

            // Add anthropic-beta headers for Messages API
            if (url.includes("/v1/messages")) {
              const betas = [
                "context-management-2025-06-27",
                "advanced-tool-use-2025-11-20",
              ];
              // Interleaved thinking only for non-adaptive thinking
              // (adaptive handles it natively; absent = user disabled)
              if (parsedBody?.thinking?.type === "enabled") {
                betas.unshift("interleaved-thinking-2025-05-14");
              }
              const existing = headers["anthropic-beta"];
              headers["anthropic-beta"] = existing
                ? [
                    ...new Set([
                      ...existing.split(",").map((s) => s.trim()),
                      ...betas,
                    ]),
                  ].join(",")
                : betas.join(",");
            }

            // Remove conflicting auth headers from SDK
            delete headers["x-api-key"];
            delete headers["authorization"];
            // Remove lowercase x-initiator from SDK's init.headers to prevent
            // case-mismatch duplicate with our title-case X-Initiator above
            delete headers["x-initiator"];

            return fetch(url, { ...init, headers });
          },
        };
      },
      methods: [
        {
          type: "oauth",
          label: "Login with OCCO",
          async authorize() {
            const deviceResponse = await fetch(
              "https://github.com/login/device/code",
              {
                method: "POST",
                headers: {
                  Accept: "application/json",
                  "Content-Type": "application/json",
                  "User-Agent": "GitHubCopilotChat/0.38.2",
                },
                body: JSON.stringify({
                  client_id: CLIENT_ID,
                  scope: "read:user",
                }),
              },
            );

            if (!deviceResponse.ok) {
              throw new Error("Failed to initiate device authorization");
            }

            const deviceData = await deviceResponse.json();
            let interval = deviceData.interval * 1000;

            return {
              url: deviceData.verification_uri,
              instructions: `Enter code: ${deviceData.user_code}`,
              method: "auto",
              callback: async () => {
                while (true) {
                  await new Promise((r) =>
                    setTimeout(r, interval + POLLING_MARGIN_MS),
                  );

                  const response = await fetch(
                    "https://github.com/login/oauth/access_token",
                    {
                      method: "POST",
                      headers: {
                        Accept: "application/json",
                        "Content-Type": "application/json",
                        "User-Agent": "GitHubCopilotChat/0.38.2",
                      },
                      body: JSON.stringify({
                        client_id: CLIENT_ID,
                        device_code: deviceData.device_code,
                        grant_type:
                          "urn:ietf:params:oauth:grant-type:device_code",
                      }),
                    },
                  );

                  if (!response.ok) return { type: "failed" };

                  const data = await response.json();

                  if (data.access_token) {
                    return {
                      type: "success",
                      refresh: data.access_token,
                      access: "",
                      expires: 0,
                    };
                  }

                  if (data.error === "authorization_pending") continue;

                  if (data.error === "slow_down") {
                    interval =
                      data.interval &&
                      typeof data.interval === "number" &&
                      data.interval > 0
                        ? data.interval * 1000
                        : interval + 5000;
                    continue;
                  }

                  if (data.error) return { type: "failed" };
                }
              },
            };
          },
        },
      ],
    },
  };
}
