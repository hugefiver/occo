/**
 * Fetch, parse, and merge Copilot models from the /models endpoint.
 *
 * Mirrors the upstream CopilotModels logic from
 * github.com/anomalyco/opencode@1fcfb69.
 */

/**
 * Minimal type guard / parser for the Copilot /models response.
 *
 * @param {unknown} value
 * @returns {Array<Record<string, unknown>>}
 */
function parseModelsResponse(value) {
  if (!value || typeof value !== "object") {
    throw new Error("Invalid models response: expected object or array");
  }
  if (Array.isArray(value)) return value.filter(isModelItem);
  if (Array.isArray(value.data)) return value.data.filter(isModelItem);
  throw new Error("Invalid models response: missing data array");
}

/**
 * @param {unknown} v
 * @returns {v is Record<string, unknown>}
 */
function isObject(v) {
  return typeof v === "object" && v !== null;
}

/**
 * @param {unknown} v
 * @returns {v is Record<string, unknown>}
 */
function isModelItem(v) {
  if (!isObject(v)) return false;
  if (typeof v.id !== "string") return false;
  if (typeof v.name !== "string") return false;
  if (typeof v.version !== "string") return false;
  if (typeof v.model_picker_enabled !== "boolean") return false;
  if (!isObject(v.capabilities)) return false;
  if (!isObject(v.capabilities.limits)) return false;
  if (typeof v.capabilities.limits.max_context_window_tokens !== "number")
    return false;
  if (typeof v.capabilities.limits.max_output_tokens !== "number")
    return false;
  if (typeof v.capabilities.limits.max_prompt_tokens !== "number")
    return false;
  if (!isObject(v.capabilities.supports)) return false;
  if (typeof v.capabilities.supports.streaming !== "boolean") return false;
  if (typeof v.capabilities.supports.tool_calls !== "boolean") return false;
  return true;
}

/**
 * Build a Model definition from a remote /models entry, preserving any
 * existing fields (name, variants, options, etc.) from the hardcoded set.
 *
 * @param {string} key
 * @param {Record<string, unknown>} remote
 * @param {string} url
 * @param {import("@opencode-ai/sdk").Model | undefined} [prev]
 * @returns {import("@opencode-ai/sdk").Model}
 */
function build(key, remote, url, prev) {
  const supports = /** @type {Record<string, unknown>} */ (
    remote.capabilities.supports
  );
  const limits = /** @type {Record<string, unknown>} */ (
    remote.capabilities.limits
  );

  const reasoning =
    !!supports.adaptive_thinking ||
    !!(
      Array.isArray(supports.reasoning_effort) &&
      supports.reasoning_effort.length
    ) ||
    supports.max_thinking_budget !== undefined ||
    supports.min_thinking_budget !== undefined;

  const vision = /** @type {Record<string, unknown> | undefined} */ (
    limits.vision
  );
  const image =
    (supports.vision === true) ||
    (Array.isArray(vision?.supported_media_types) &&
      vision.supported_media_types.some((/** @type {string} */ item) =>
        item.startsWith("image/"),
      ));
  const isMessagesApi =
    Array.isArray(remote.supported_endpoints) &&
    remote.supported_endpoints.includes("/v1/messages");

  return {
    id: key,
    providerID: "occo",
    api: {
      id: String(remote.id),
      url: isMessagesApi ? `${url}/v1` : url,
      npm: isMessagesApi
        ? "@ai-sdk/anthropic"
        : "@ai-sdk/github-copilot",
    },
    status: "active",
    limit: {
      context: Number(limits.max_context_window_tokens),
      input: Number(limits.max_prompt_tokens),
      output: Number(limits.max_output_tokens),
    },
    capabilities: {
      temperature: prev?.capabilities?.temperature ?? true,
      reasoning: prev?.capabilities?.reasoning ?? reasoning,
      attachment: prev?.capabilities?.attachment ?? true,
      toolcall: supports.tool_calls === true,
      input: {
        text: true,
        audio: false,
        image,
        video: false,
        pdf: false,
      },
      output: {
        text: true,
        audio: false,
        image: false,
        video: false,
        pdf: false,
      },
      interleaved: prev?.capabilities?.interleaved ?? false,
    },
    family: prev?.family ?? String(remote.capabilities.family),
    name: prev?.name ?? String(remote.name),
    cost: {
      input: 0,
      output: 0,
      cache: { read: 0, write: 0 },
    },
    options: prev?.options ?? {},
    headers: prev?.headers ?? {},
    release_date:
      prev?.release_date ??
      (String(remote.version).startsWith(`${remote.id}-`)
        ? String(remote.version).slice(String(remote.id).length + 1)
        : String(remote.version)),
    variants: prev?.variants ?? {},
  };
}

/**
 * Fetch models from the Copilot /models endpoint and merge them with the
 * existing hardcoded definitions.
 *
 * @param {string} baseURL
 * @param {HeadersInit} [headers]
 * @param {Record<string, import("@opencode-ai/sdk").Model>} [existing]
 * @returns {Promise<Record<string, import("@opencode-ai/sdk").Model>>}
 */
export async function getModels(baseURL, headers = {}, existing = {}) {
  const res = await fetch(`${baseURL}/models`, {
    headers,
    signal: AbortSignal.timeout(5_000),
  });
  if (!res.ok) {
    throw new Error(`Failed to fetch models: ${res.status}`);
  }
  const data = parseModelsResponse(await res.json());

  const result = { ...existing };
  /** @type {Map<string, Record<string, unknown>>} */
  const remoteMap = new Map();
  for (const m of data) {
    const policy = /** @type {Record<string, unknown> | undefined} */ (
      m.policy
    );
    if (m.model_picker_enabled === true && policy?.state !== "disabled") {
      remoteMap.set(String(m.id), m);
    }
  }
  if (remoteMap.size === 0) {
    throw new Error("No enabled Copilot models returned");
  }

  // Prune existing models whose api.id is no longer returned by the endpoint.
  // Falls back to matching by result key when api.id is absent (hardcoded MODELS).
  for (const [key, model] of Object.entries(result)) {
    const apiId = model.api?.id;
    const m = apiId ? remoteMap.get(apiId) : remoteMap.get(key);
    if (!m) {
      delete result[key];
      continue;
    }
    result[key] = build(key, m, baseURL, model);
  }

  // Add brand-new endpoint models not already keyed in result
  for (const [id, m] of remoteMap) {
    if (id in result) continue;
    result[id] = build(id, m, baseURL);
  }
  if (Object.keys(result).length === 0) {
    throw new Error("No Copilot models available after merging response");
  }

  return result;
}
