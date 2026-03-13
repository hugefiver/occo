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
    "User-Agent": "GitHubCopilotChat/0.39.0",
    "Editor-Version": "vscode/1.111.0",
    "Editor-Plugin-Version": "copilot-chat/0.39.0",
    "Copilot-Integration-Id": "vscode-chat",
    "X-GitHub-Api-Version": "2025-05-01",
  };

  // Resolved dynamically from token response's endpoints.api (unless legacy mode)
  let resolvedApiUrl = DEFAULT_API_URL;

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

  // Claude variants: commented out, using copilot SDK auto-config via ProviderTransform.variants()
  // const CLAUDE_OPUS_VARIANTS = {
  //   default: { thinking_budget: 8000 },
  //   thinking: { thinking_budget: 8000 },
  //   max: { thinking_budget: 32000 },
  // };
  // const CLAUDE_SONNET_VARIANTS = {
  //   thinking: { thinking_budget: 4000 },
  //   max: { thinking_budget: 32000 },
  // };
  // const CLAUDE_HAIKU_VARIANTS = {
  //   thinking: { thinking_budget: 2000 },
  //   max: { thinking_budget: 32000 },
  // };

  // GPT reasoning effort variants (for mini models): default=high
  const GPT_REASONING_VARIANTS = {
    default: {
      reasoningEffort: "high",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
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

  // GPT Codex variants: only high (default) and xhigh
  const GPT_CODEX_VARIANTS = {
    default: {
      reasoningEffort: "high",
      reasoningSummary: "auto",
      include: ["reasoning.encrypted_content"],
    },
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
  //   claude         → auto (copilot SDK ProviderTransform.variants())
  //   gemini         → no variants
  //   gpt-5.1, gpt-5.2, gpt-5.4 (non-codex) → default(high) / low/medium/high/xhigh
  //   gpt-5.*-codex/codex-max → default(high) / high/xhigh only
  //   gpt-5-mini, gpt-5.1-codex-mini → no variants (mini = no reasoning)
  //   gpt-4o/4.1    → no variants (no reasoning)
  //   grok           → no variants
  //   oswe/raptor    → no variants (mini = no reasoning)
  // ---------------------------------------------------------------------------

  const MODELS = {
    // --- Claude models ---
    "claude-opus-4.6-fast": {
      name: "Claude Opus 4.6 (fast mode)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 192000, output: 64000 },
    },
    "claude-opus-4.6": {
      name: "Claude Opus 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 192000, output: 64000 },
    },
    "claude-opus-4.5": {
      name: "Claude Opus 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
    },
    "claude-sonnet-4.6": {
      name: "Claude Sonnet 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
    },
    "claude-sonnet-4.5": {
      name: "Claude Sonnet 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
    },
    "claude-sonnet-4": {
      name: "Claude Sonnet 4",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 144000, output: 16000 },
    },
    "claude-haiku-4.5": {
      name: "Claude Haiku 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 160000, output: 32000 },
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
    // Fallback: ensure X-Interaction-Type is set for all chat requests
    // -------------------------------------------------------------------------
    "chat.headers": async (incoming, output) => {
      if (incoming.model.providerID !== "occo") return;
      if (!output.headers["X-Interaction-Type"]) {
        output.headers["X-Interaction-Type"] = "conversation-subagent";
      }
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
        await refreshTokenIfNeeded(getAuth);

        // Persistent task ID for all requests in this session
        const sessionTaskId = crypto.randomUUID();

        // Zero out costs (Copilot is free)
        if (provider?.models) {
          for (const model of Object.values(provider.models)) {
            model.cost = { input: 0, output: 0, cache: { read: 0, write: 0 } };
          }
        }

        return {
          baseURL: USE_TOKEN_ENDPOINT_API ? resolvedApiUrl : DEFAULT_API_URL,
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

            // Detect vision requests
            // All Copilot models (including Claude) use OpenAI-compatible format
            let isVision = false;
            try {
              // Completions API: body.messages
              if (parsedBody?.messages) {
                isVision = parsedBody.messages.some(
                  (msg) =>
                    Array.isArray(msg.content) &&
                    msg.content.some((part) => part.type === "image_url"),
                );
              }

              // Responses API: body.input
              if (parsedBody?.input) {
                isVision = parsedBody.input.some(
                  (item) =>
                    Array.isArray(item?.content) &&
                    item.content.some((part) => part.type === "input_image"),
                );
              }
            } catch {}

            const requestId = crypto.randomUUID();
            const headers = {
              ...init.headers,
              ...HEADERS,
              Authorization: `Bearer ${info.access}`,
              "OpenAI-Intent": "conversation-panel",
              "X-Interaction-Type": "conversation-subagent",
              "X-Request-Id": requestId,
              "X-Agent-Task-Id": sessionTaskId,
            };
            if (isVision) {
              headers["Copilot-Vision-Request"] = "true";
            }

            // Remove conflicting auth headers from SDK
            delete headers["x-api-key"];
            delete headers["authorization"];

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
                  "User-Agent": "GitHubCopilotChat/0.39.0",
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
                        "User-Agent": "GitHubCopilotChat/0.39.0",
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
