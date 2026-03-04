/**
 * @type {import('@opencode-ai/plugin').Plugin}
 */
export async function OccoAuthPlugin({ client }) {
  const CLIENT_ID = "Iv1.b507a08c87ecfe98";
  const BASE_URL = "https://api.githubcopilot.com";
  const TOKEN_URL = "https://api.github.com/copilot_internal/v2/token";
  const POLLING_MARGIN_MS = 3000;
  const HEADERS = {
    "User-Agent": "GitHubCopilotChat/0.37.8",
    "Editor-Version": "vscode/1.109.5",
    "Editor-Plugin-Version": "copilot-chat/0.37.8",
    "Copilot-Integration-Id": "vscode-chat",
  };

  // ---------------------------------------------------------------------------
  // Hardcoded model definitions (avoids network fetch during plugin init which
  // can hang in restricted network environments and block opencode startup)
  // ---------------------------------------------------------------------------

  // Variant presets for Copilot models
  const CLAUDE_VARIANTS = {
    thinking: { thinking_budget: 4000 },
  };
  const GPT_REASONING_VARIANTS = Object.fromEntries(
    ["low", "medium", "high"].map((effort) => [
      effort,
      {
        reasoningEffort: effort,
        reasoningSummary: "auto",
        include: ["reasoning.encrypted_content"],
      },
    ]),
  );
  const GPT_REASONING_XHIGH_VARIANTS = Object.fromEntries(
    ["low", "medium", "high", "xhigh"].map((effort) => [
      effort,
      {
        reasoningEffort: effort,
        reasoningSummary: "auto",
        include: ["reasoning.encrypted_content"],
      },
    ]),
  );

  const MODELS = {
    // --- Claude models ---
    "claude-haiku-4.5": {
      name: "Claude Haiku 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
      variants: CLAUDE_VARIANTS,
    },
    "claude-opus-4.5": {
      name: "Claude Opus 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
      variants: CLAUDE_VARIANTS,
    },
    "claude-opus-4.6": {
      name: "Claude Opus 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
      variants: CLAUDE_VARIANTS,
    },
    "claude-opus-4.6-fast": {
      name: "Claude Opus 4.6 (fast mode)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
      variants: CLAUDE_VARIANTS,
    },
    "claude-sonnet-4": {
      name: "Claude Sonnet 4",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 16000 },
      variants: CLAUDE_VARIANTS,
    },
    "claude-sonnet-4.5": {
      name: "Claude Sonnet 4.5",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
      variants: CLAUDE_VARIANTS,
    },
    "claude-sonnet-4.6": {
      name: "Claude Sonnet 4.6",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 32000 },
      variants: CLAUDE_VARIANTS,
    },
    // --- Gemini models (no variants — Copilot only returns thinking, no effort control) ---
    "gemini-2.5-pro": {
      name: "Gemini 2.5 Pro",
      reasoning: true,
      tool_call: true,
      temperature: false,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
    },
    "gemini-3-flash": {
      name: "Gemini 3 Flash (Preview)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 109000, output: 64000 },
    },
    "gemini-3-pro": {
      name: "Gemini 3 Pro (Preview)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 109000, output: 64000 },
    },
    "gemini-3.1-pro": {
      name: "Gemini 3.1 Pro (Preview)",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 109000, output: 64000 },
    },
    // --- GPT models ---
    "gpt-4.1": {
      name: "GPT-4.1",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 64000, output: 16384 },
    },
    "gpt-4o": {
      name: "GPT-4o",
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 64000, output: 4096 },
    },
    "gpt-5-mini": {
      name: "GPT-5 mini",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
      variants: GPT_REASONING_VARIANTS,
    },
    "gpt-5.1": {
      name: "GPT-5.1",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 64000 },
      variants: GPT_REASONING_VARIANTS,
    },
    "gpt-5.1-codex": {
      name: "GPT-5.1-Codex",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 128000 },
      variants: GPT_REASONING_VARIANTS,
    },
    "gpt-5.1-codex-max": {
      name: "GPT-5.1-Codex-Max",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 128000 },
      variants: GPT_REASONING_XHIGH_VARIANTS,
    },
    "gpt-5.1-codex-mini": {
      name: "GPT-5.1-Codex-Mini",
      reasoning: true,
      tool_call: true,
      temperature: true,
      modalities: { input: ["text", "image"], output: ["text"] },
      limit: { context: 128000, output: 128000 },
      variants: GPT_REASONING_VARIANTS,
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
    "raptor-mini": {
      name: "Raptor mini",
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
        api: BASE_URL,
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
          baseURL: BASE_URL,
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

            // Detect agent calls and vision requests
            // All Copilot models (including Claude) use OpenAI-compatible format
            let isAgent = true;
            let isVision = false;
            try {
              const body =
                typeof init.body === "string"
                  ? JSON.parse(init.body)
                  : init.body;

              // Completions API: body.messages
              if (body?.messages) {
                const last = body.messages[body.messages.length - 1];
                isAgent =
                  last?.role !== "user" ||
                  (body?.messages)
                    .map((x) => (x?.role === "user" ? 1 : 0))
                    .reduce((acc, x) => acc + x) > 1;
                isVision = body.messages.some(
                  (msg) =>
                    Array.isArray(msg.content) &&
                    msg.content.some((part) => part.type === "image_url"),
                );
              }

              // Responses API: body.input
              if (body?.input) {
                const last = body.input[body.input.length - 1];
                isAgent =
                  last?.role !== "user" ||
                  (body?.input)
                    .map((x) => (x?.role === "user" ? 1 : 0))
                    .reduce((acc, x) => acc + x) > 1;
                isVision = body.input.some(
                  (item) =>
                    Array.isArray(item?.content) &&
                    item.content.some((part) => part.type === "input_image"),
                );
              }
            } catch {}

            const headers = {
              ...init.headers,
              ...HEADERS,
              Authorization: `Bearer ${info.access}`,
              "Openai-Intent": "conversation-edits",
              "X-Initiator": isAgent ? "agent" : "user",
            };
            if (isVision) {
              headers["Copilot-Vision-Request"] = "true";
            }

            // Remove conflicting auth headers from SDK
            delete headers["x-api-key"];
            delete headers["authorization"];

            return fetch(input, { ...init, headers });
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
                  "User-Agent": "GitHubCopilotChat/0.37.8",
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
                        "User-Agent": "GitHubCopilotChat/0.37.8",
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
