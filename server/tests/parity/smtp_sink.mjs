// A scriptable SMTP sink for the mail parity checks (tests/parity/admin_settings.py).
//
// The same file runs next to each server: on the host for the Rust server and
// inside the reference app container (which has Node) for the original, both
// listening on 127.0.0.1, so a mail configuration naming 127.0.0.1:<port> means
// the same thing to both and they can be compared answer for answer. No
// dependencies; never prints credentials.
//
//   node smtp_sink.mjs <log file> <base port> [lifetime seconds]
//
// Ports (base + n):
//   0  ESMTP: PIPELINING, 8BITMIME, AUTH PLAIN LOGIN
//   1  greets with 554 (and keeps the connection open)
//   2  never greets
//   3  accepts and closes at once
//   4  offers STARTTLS, accepts it, then hangs up on the TLS handshake
//   5  refuses EHLO (HELO works)
//   6  AUTH LOGIN only
//   7  AUTH CRAM-MD5 only
//
// Credentials: user "good-user" with password "good-pass"; anything else is 535.
// Recipients: a local part starting "reject-data" is accepted and the message
// refused with 554; "reject" gets 550; "defer" gets 451. A sender starting
// "reject-from" gets 553. Every finished DATA phase is logged as one JSON line.

import net from "node:net";
import crypto from "node:crypto";
import fs from "node:fs";

const [logFile, basePortArg, lifetimeArg] = process.argv.slice(2);
const basePort = Number(basePortArg || 2525);
const lifetime = Number(lifetimeArg || 3600);

const GOOD_USER = "good-user";
const GOOD_PASS = "good-pass";

function log(record) {
  fs.appendFileSync(logFile, JSON.stringify(record) + "\n");
}

function local(address) {
  return address.replace(/^<|>$/g, "").split("@")[0].toLowerCase();
}

function serve(mode, port) {
  const server = net.createServer((socket) => {
    socket.on("error", () => {});
    if (mode === "close") {
      socket.destroy();
      return;
    }
    if (mode === "silent") return;
    if (mode === "greet554") {
      socket.write("554 5.3.2 sink.test Service currently unavailable\r\n");
      return;
    }

    const session = { helo: null, auth: null, from: null, to: [], rejected: [], data: null, pending: null };
    let buffer = "";
    let inData = false;
    let dataLines = [];
    let tlsPending = false;
    const send = (line) => socket.write(line + "\r\n");

    const ehloLines = () => {
      const lines = ["sink.test"];
      if (mode === "esmtp") lines.push("PIPELINING", "8BITMIME", "AUTH PLAIN LOGIN");
      if (mode === "starttls") lines.push("STARTTLS", "AUTH PLAIN LOGIN");
      if (mode === "helo") lines.push("AUTH PLAIN LOGIN");
      if (mode === "login") lines.push("AUTH LOGIN");
      if (mode === "cram") lines.push("AUTH CRAM-MD5");
      return lines;
    };

    const finishAuth = (method, user, pass) => {
      const ok = user === GOOD_USER && pass === GOOD_PASS;
      session.auth = { method, user, ok };
      send(ok ? "235 2.7.0 Authentication successful" : "535 5.7.8 Error: authentication failed: bad credentials");
    };

    const handle = (line) => {
      if (session.pending) {
        const step = session.pending;
        session.pending = null;
        step(line);
        return;
      }
      const [verb, ...rest] = line.split(" ");
      const arg = rest.join(" ");
      switch (verb.toUpperCase()) {
        case "EHLO":
          if (mode === "helo") return send("502 5.5.2 Error: command not recognized");
          session.helo = arg;
          {
            const lines = ehloLines();
            lines.forEach((l, i) => send((i === lines.length - 1 ? "250 " : "250-") + l));
          }
          return;
        case "HELO":
          session.helo = arg;
          return send("250 sink.test");
        case "STARTTLS":
          if (mode !== "starttls") return send("454 4.7.0 TLS not available due to local problem");
          tlsPending = true;
          return send("220 2.0.0 Ready to start TLS");
        case "AUTH": {
          const [mech, initial] = arg.split(" ");
          const m = (mech || "").toUpperCase();
          const b64 = (s) => Buffer.from(s || "", "base64").toString("utf8");
          if (m === "PLAIN") {
            const check = (payload) => {
              const [, user, pass] = b64(payload).split("\0");
              finishAuth("PLAIN", user, pass);
            };
            if (initial) return check(initial);
            session.pending = check;
            return send("334 ");
          }
          if (m === "LOGIN") {
            session.pending = (u) => {
              session.pending = (p) => finishAuth("LOGIN", b64(u), b64(p));
              send("334 UGFzc3dvcmQ6");
            };
            return send("334 VXNlcm5hbWU6");
          }
          if (m === "CRAM-MD5") {
            const challenge = `<${crypto.randomBytes(6).toString("hex")}@sink.test>`;
            session.pending = (answer) => {
              const [user, digest] = b64(answer).split(" ");
              const expected = crypto.createHmac("md5", GOOD_PASS).update(challenge).digest("hex");
              finishAuth("CRAM-MD5", user, user === GOOD_USER && digest === expected ? GOOD_PASS : "");
            };
            return send("334 " + Buffer.from(challenge).toString("base64"));
          }
          return send("504 5.5.4 Unrecognized authentication type");
        }
        case "MAIL": {
          const from = arg.replace(/^FROM:/i, "").trim();
          if (local(from).startsWith("reject-from")) return send("553 5.7.1 <" + from + ">: Sender address rejected: not owned by user");
          session.from = from;
          session.to = [];
          session.rejected = [];
          return send("250 2.1.0 Ok");
        }
        case "RCPT": {
          const to = arg.replace(/^TO:/i, "").trim();
          const l = local(to);
          if (l.startsWith("reject-data")) {
            session.to.push(to);
            return send("250 2.1.5 Ok");
          }
          if (l.startsWith("reject")) {
            session.rejected.push(to);
            return send("550 5.1.1 " + to + ": Recipient address rejected: User unknown in local recipient table");
          }
          if (l.startsWith("defer")) {
            session.rejected.push(to);
            return send("451 4.3.0 " + to + ": Temporary lookup failure");
          }
          session.to.push(to);
          return send("250 2.1.5 Ok");
        }
        case "DATA":
          inData = true;
          dataLines = [];
          return send("354 End data with <CR><LF>.<CR><LF>");
        case "RSET":
          session.from = null;
          session.to = [];
          return send("250 2.0.0 Ok");
        case "NOOP":
          return send("250 2.0.0 Ok");
        case "QUIT":
          send("221 2.0.0 Bye");
          return socket.end();
        default:
          return send("502 5.5.2 Error: command not recognized");
      }
    };

    socket.on("data", (chunk) => {
      if (tlsPending) {
        // The client's TLS handshake: hang up on it.
        socket.destroy();
        return;
      }
      buffer += chunk.toString("binary");
      let index;
      while ((index = buffer.indexOf("\r\n")) >= 0) {
        const line = Buffer.from(buffer.slice(0, index), "binary").toString("utf8");
        buffer = buffer.slice(index + 2);
        if (inData) {
          if (line === ".") {
            inData = false;
            const refused = session.to.some((t) => local(t).startsWith("reject-data"));
            log({
              port,
              helo: session.helo !== null,
              auth: session.auth,
              from: session.from,
              to: session.to,
              rejected: session.rejected,
              refused,
              data: dataLines.join("\r\n"),
            });
            send(refused ? "554 5.7.1 Message rejected as spam" : "250 2.0.0 Ok: queued as SINK");
          } else {
            dataLines.push(line.startsWith("..") ? line.slice(1) : line);
          }
          continue;
        }
        handle(line);
        if (socket.destroyed) return;
      }
    });

    send("220 sink.test ESMTP parity sink");
  });
  server.listen(port, "127.0.0.1");
  return server;
}

const modes = ["esmtp", "greet554", "silent", "close", "starttls", "helo", "login", "cram"];
modes.forEach((mode, i) => serve(mode, basePort + i));
fs.writeFileSync(logFile, "");
fs.writeFileSync(logFile + ".pid", String(process.pid));
setTimeout(() => process.exit(0), lifetime * 1000);
