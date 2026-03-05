/**
 * @type {import('@opencode-ai/plugin').Plugin}
 */
export async function OccoAuthPlugin({ client }) {
  const CLIENT_ID = "Iv1.b507a08c87ecfe98";
  const DEFAULT_API_URL = "https://api.githubcopilot.com";
  const MODEL_LAB_URL = "https://api-model-lab.githubcopilot.com";
  const TOKEN_URL = "https://api.github.com/copilot_internal/v2/token";
  const POLLING_MARGIN_MS = 3000;
  const HEADERS = {
    "User-Agent": "GitHubCopilotChat/0.38.0",
    "Editor-Version": "vscode/1.110.0",
    "Editor-Plugin-Version": "copilot-chat/0.38.0",
    "Copilot-Integration-Id": "vscode-chat",
    "X-GitHub-Api-Version": "2025-05-01",
  };

  // Resolved dynamically from token response's endpoints.api
  let resolvedApiUrl = DEFAULT_API_URL;

  // Model IDs served from Model Lab (preview/experimental models).
  // These route to api-model-lab.githubcopilot.com instead of endpoints.api.
  const MODEL_LAB_IDS = new Set([
    "claude-opus-4.6-fast",
    "gemini-3-flash-preview",
    "gemini-3-pro-preview",
    "gemini-3.1-pro-preview",
    "gpt-5.1-codex-mini",
    "oswe-vscode-prime",
  ]);

  // ---------------------------------------------------------------------------
  // Hardcoded model definitions from Copilot API.
  // Variants are NOT set here — opencode's ProviderTransform.variants()
  // computes them automatically based on model ID and @ai-sdk/github-copilot:
  //   claude  → { thinking: { thinking_budget: 4000 } }
  //   gemini  → {} (no variants)
  //   gpt 5.1-codex-max/5.2/5.3 → low/medium/high/xhigh
  //   other gpt → low/medium/high
  //   grok    → {} (no variants)
  // ---------------------------------------------------------------------------

  const MODELS = {
    // --- Claude models ---
    "claude-opus-4.6-fast": {
      name: "Claude Opus 4.6 (fast mode)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "claude-opus-4.6": {
      name: "Claude Opus 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "claude-opus-4.5": {
      name: "Claude Opus 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
    },
    "claude-sonnet-4.6": {
      name: "Claude Sonnet 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
    },
    "claude-sonnet-4.5": {
      name: "Claude Sonnet 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
    },
    "claude-sonnet-4": {
      name: "Claude Sonnet 4",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 16000 },
    },
    "claude-haiku-4.5": {
      name: "Claude Haiku 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
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
      limit: { context: 64000, output: 4096 },
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
      limit: { context: 128000, output: 64000 },
    },
    "gpt-5.1": {
      name: "GPT-5.1",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "gpt-5.1-codex": {
      name: "GPT-5.1-Codex",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 128000 },
    },
    "gpt-5.1-codex-mini": {
      name: "GPT-5.1-Codex-Mini",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 128000 },
    },
    "gpt-5.1-codex-max": {
      name: "GPT-5.1-Codex-Max",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 128000 },
    },
    "gpt-5.2": {
      name: "GPT-5.2",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "gpt-5.2-codex": {
      name: "GPT-5.2-Codex",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 272000, output: 128000 },
    },
    "gpt-5.3-codex": {
      name: "GPT-5.3-Codex",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 272000, output: 128000 },
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
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 200000, output: 64000 },
    },
  };

  return {
    // -------------------------------------------------------------------------
    // Register "occo" as a provider with Copilot model definitions
    // -------------------------------------------------------------------------
    config: (config) => {
      config.provider = config.provider ?? {};
      config.provider.occo = {
        name: "GitHub Copilot (OCCO)",
        npm: "@ai-sdk/github-copilot",
        api: DEFAULT_API_URL,
        models: MODELS,
      };
    },

    // -------------------------------------------------------------------------
    // Subagent quota protection: mark subagent sessions with x-initiator header
    // so they don't consume user's Copilot quota
    // -------------------------------------------------------------------------
    "chat.headers": async (incoming, output) => {
      if (incoming.model.providerID !== "occo") return;
      try {
        const session = await client.session.get({
          path: { id: incoming.sessionID },
        });
        if (session.data?.parentID) {
          output.headers["x-initiator"] = "agent";
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

        // Zero out costs (Copilot is free)
        if (provider?.models) {
          for (const model of Object.values(provider.models)) {
            model.cost = { input: 0, output: 0, cache: { read: 0, write: 0 } };
          }
        }

        return {
          baseURL: resolvedApiUrl,
          apiKey: "",
          async fetch(input, init) {
            const info = await getAuth();
            if (info.type !== "oauth") return fetch(input, init);

            // Get or refresh the Copilot session token
            if (!info.access || info.expires < Date.now()) {
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

              // Dynamically resolve API URL from token response
              if (tokenData.endpoints?.api) {
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
            }

            // Parse request body once for routing + detection
            let parsedBody = null;
            try {
              parsedBody =
                typeof init.body === "string"
                  ? JSON.parse(init.body)
                  : init.body;
            } catch {}

            // Rewrite request URL based on model source:
            // - Model Lab models → api-model-lab.githubcopilot.com
            // - Prod models → resolved endpoints.api URL
            let url = typeof input === "string" ? input : input.url;
            const requestModelId = parsedBody?.model;

            if (requestModelId && MODEL_LAB_IDS.has(requestModelId)) {
              // Route to Model Lab
              url = url.replace(DEFAULT_API_URL, MODEL_LAB_URL);
              if (resolvedApiUrl !== DEFAULT_API_URL) {
                url = url.replace(resolvedApiUrl, MODEL_LAB_URL);
              }
            } else if (url.startsWith(DEFAULT_API_URL) && resolvedApiUrl !== DEFAULT_API_URL) {
              // Route to resolved prod API
              url = url.replace(DEFAULT_API_URL, resolvedApiUrl);
            }

            // Detect agent calls and vision requests
            // All Copilot models (including Claude) use OpenAI-compatible format
            let isAgent = true;
            let isVision = false;
            try {
              // Completions API: body.messages
              if (parsedBody?.messages) {
                const last = parsedBody.messages[parsedBody.messages.length - 1];
                isAgent =
                  last?.role !== "user" ||
                  (parsedBody?.messages)
                    .map((x) => (x?.role === "user" ? 1 : 0))
                    .reduce((acc, x) => acc + x) > 1;
                isVision = parsedBody.messages.some(
                  (msg) =>
                    Array.isArray(msg.content) &&
                    msg.content.some((part) => part.type === "image_url"),
                );
              }

              // Responses API: body.input
              if (parsedBody?.input) {
                const last = parsedBody.input[parsedBody.input.length - 1];
                isAgent =
                  last?.role !== "user" ||
                  (parsedBody?.input)
                    .map((x) => (x?.role === "user" ? 1 : 0))
                    .reduce((acc, x) => acc + x) > 1;
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
              "X-Initiator": isAgent ? "agent" : "user",
              "X-Request-Id": requestId,
              "X-Agent-Task-Id": requestId,
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
          label: "Login with GitHub Copilot (OCCO)",
          async authorize() {
            const deviceResponse = await fetch(
              "https://github.com/login/device/code",
              {
                method: "POST",
                headers: {
                  Accept: "application/json",
                  "Content-Type": "application/json",
                  "User-Agent": "GitHubCopilotChat/0.38.0",
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
                        "User-Agent": "GitHubCopilotChat/0.38.0",
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
