export interface Env {
  SHELLDECK_KV: KVNamespace;
}

interface PlatformRelease {
  url: string;
  sha256: string;
  size: number;
}

interface UpdateManifest {
  version: string;
  pub_date: string;
  platforms: Record<string, PlatformRelease>;
}

const CORS_HEADERS: Record<string, string> = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
};

function jsonResponse(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "Content-Type": "application/json",
      ...CORS_HEADERS,
    },
  });
}

function textResponse(text: string, status = 200): Response {
  return new Response(text, {
    status,
    headers: {
      "Content-Type": "text/plain",
      ...CORS_HEADERS,
    },
  });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    // Handle CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, { status: 204, headers: CORS_HEADERS });
    }

    if (url.pathname === "/health") {
      return textResponse("ok");
    }

    if (url.pathname === "/api/releases/latest") {
      const platform = url.searchParams.get("platform");
      if (!platform) {
        return jsonResponse({ error: "Missing 'platform' query parameter" }, 400);
      }

      const raw = await env.SHELLDECK_KV.get("latest-release");
      if (!raw) {
        return jsonResponse({ error: "No release manifest found" }, 404);
      }

      let manifest: UpdateManifest;
      try {
        manifest = JSON.parse(raw);
      } catch {
        return jsonResponse({ error: "Corrupt manifest data" }, 500);
      }

      const platformData = manifest.platforms[platform];
      if (!platformData) {
        return jsonResponse(
          { error: `No release available for platform '${platform}'` },
          404
        );
      }

      return jsonResponse({
        version: manifest.version,
        url: platformData.url,
        sha256: platformData.sha256,
        size: platformData.size,
        pub_date: manifest.pub_date,
      });
    }

    return textResponse("Not Found", 404);
  },
};
