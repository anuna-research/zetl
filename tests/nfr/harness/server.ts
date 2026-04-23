// Minimal static-file server for the NFR harness. Avoids external deps — we
// rely only on Node's standard library so CI doesn't need to install a server
// package. Serves exactly one directory tree and responds with sensible MIME
// types for the handful of extensions ztl's dist/ emits.

import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { extname, join, normalize, resolve, sep } from "node:path";

export const DEFAULT_HARNESS_PORT = 4873;

const MIME: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".mjs": "application/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".webp": "image/webp",
  ".woff": "font/woff",
  ".woff2": "font/woff2",
  ".ttf": "font/ttf",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
  ".map": "application/json; charset=utf-8",
};

export interface StartedServer {
  server: Server;
  port: number;
  url: string;
  stop(): Promise<void>;
}

export interface ServeOptions {
  root: string;
  port?: number;
  /** Host to bind to. Default 127.0.0.1. */
  host?: string;
}

export async function serveDist(opts: ServeOptions): Promise<StartedServer> {
  const root = resolve(opts.root);
  const host = opts.host ?? "127.0.0.1";
  const port = opts.port ?? DEFAULT_HARNESS_PORT;

  const server = createServer((req, res) => handle(req, res, root));

  await new Promise<void>((resolvePromise, reject) => {
    server.once("error", reject);
    server.listen(port, host, () => {
      server.off("error", reject);
      resolvePromise();
    });
  });

  const addr = server.address();
  const actualPort = typeof addr === "object" && addr ? addr.port : port;

  return {
    server,
    port: actualPort,
    url: `http://${host}:${actualPort}`,
    stop: () =>
      new Promise((resolvePromise, reject) =>
        server.close((err) => (err ? reject(err) : resolvePromise())),
      ),
  };
}

async function handle(req: IncomingMessage, res: ServerResponse, root: string): Promise<void> {
  try {
    const url = new URL(req.url ?? "/", "http://internal");
    let pathname = decodeURIComponent(url.pathname);
    if (pathname.endsWith("/")) pathname += "index.html";

    // Resolve against root, reject path traversal.
    const requested = normalize(join(root, pathname));
    if (!requested.startsWith(root + sep) && requested !== root) {
      res.writeHead(403).end("forbidden");
      return;
    }

    let target = requested;
    let info = await safeStat(target);

    // If the path has no extension and isn't a file, try `${path}.html` — the
    // shape `ztl build` emits pages under.
    if (!info && !extname(requested)) {
      const htmlCandidate = `${requested}.html`;
      info = await safeStat(htmlCandidate);
      if (info?.isFile()) target = htmlCandidate;
    }

    if (!info) {
      res.writeHead(404, { "content-type": "text/plain; charset=utf-8" }).end("not found");
      return;
    }

    if (info.isDirectory()) {
      const indexPath = join(target, "index.html");
      const indexInfo = await safeStat(indexPath);
      if (!indexInfo?.isFile()) {
        res.writeHead(404).end("not found");
        return;
      }
      target = indexPath;
      info = indexInfo;
    }

    const mime = MIME[extname(target).toLowerCase()] ?? "application/octet-stream";
    res.writeHead(200, {
      "content-type": mime,
      "content-length": info.size,
      // Prevent browser caching so cold-load measurements are stable.
      "cache-control": "no-store, must-revalidate",
    });
    createReadStream(target).pipe(res);
  } catch (err) {
    res.writeHead(500).end(`internal error: ${(err as Error).message}`);
  }
}

async function safeStat(path: string) {
  try {
    return await stat(path);
  } catch {
    return null;
  }
}
