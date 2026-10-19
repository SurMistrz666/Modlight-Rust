// A real tus client (tus-js-client 4.3.1, the version Uppy uses in the panel,
// from server/web's dependencies) against both servers: create, interrupt
// after the first chunk, resume from the server's offset, finish; then a
// second file resumed from a fresh client with the stored upload URL (what a
// reloaded tab or the desktop app does).
//
//   node server/tests/parity/tus_client.cjs <base url> <token file> [<base url> <token file> ...]
//
// Prints one JSON line per server: what the client saw (status of every
// request, the final offset, the X-Media headers with the origin replaced).
// The caller compares the lines. Never prints the token.

const path = require("node:path");
const fs = require("node:fs");
const tus = require(path.join(
  __dirname,
  "../../web/node_modules/.pnpm/tus-js-client@4.3.1/node_modules/tus-js-client",
));

function run(base, token, data, name, { interruptAfter } = {}) {
  return new Promise((resolve, reject) => {
    const seen = [];
    let media = null;
    let interrupted = false;
    const upload = new tus.Upload(data, {
      endpoint: base + "/api/files",
      chunkSize: 1024 * 1024,
      retryDelays: [0, 100, 200],
      headers: { "x-api-key": token },
      metadata: { filename: name, filetype: "application/octet-stream" },
      uploadSize: data.length,
      onAfterResponse(req, res) {
        seen.push(req.getMethod() + " " + res.getStatus());
        const url = res.getHeader("X-Media-Url");
        if (url) {
          media = {
            url: url.replace(/^http:\/\/[^/]+/, "<origin>"),
            raw: (res.getHeader("X-Media-Raw-Url") || "").replace(/^http:\/\/[^/]+/, "<origin>"),
            copy: (res.getHeader("X-Media-Copy-Url") || "").replace(/^http:\/\/[^/]+/, "<origin>"),
          };
        }
      },
      onChunkComplete(_size, accepted) {
        if (interruptAfter && !interrupted && accepted >= interruptAfter) {
          interrupted = true;
          upload.abort().then(() => {
            // A brand-new client resumes from the stored URL.
            const again = new tus.Upload(data, {
              uploadUrl: upload.url,
              chunkSize: 1024 * 1024,
              headers: { "x-api-key": token },
              uploadSize: data.length,
              onAfterResponse(req, res) {
                seen.push("resume " + req.getMethod() + " " + res.getStatus());
                const url = res.getHeader("X-Media-Url");
                if (url) media = { url: url.replace(/^http:\/\/[^/]+/, "<origin>") };
              },
              onError: reject,
              onSuccess() {
                resolve({ seen, media, offset: data.length });
              },
            });
            again.start();
          });
        }
      },
      onError: reject,
      onSuccess() {
        resolve({ seen, media, offset: data.length });
      },
    });
    upload.start();
  });
}

(async () => {
  const args = process.argv.slice(2);
  // 3.5 MiB of deterministic bytes.
  const data = Buffer.alloc(3.5 * 1024 * 1024);
  for (let i = 0; i < data.length; i++) data[i] = (i * 31 + 7) & 0xff;
  for (let i = 0; i < args.length; i += 2) {
    const base = args[i];
    const token = fs.readFileSync(args[i + 1], "utf8").trim();
    try {
      const plain = await run(base, token, data, "tus-client.bin");
      const resumed = await run(base, token, data.subarray(0, 2.5 * 1024 * 1024), "resumed.bin", {
        interruptAfter: 1024 * 1024,
      });
      console.log(JSON.stringify({ plain, resumed }));
    } catch (err) {
      console.log(JSON.stringify({ error: String(err && err.message ? err.message : err) }));
    }
  }
})();
