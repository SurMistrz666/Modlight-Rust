// `server-only` and `client-only` are markers that fail a build when a module
// lands on the wrong side. In the browser build the server side does not exist,
// so both resolve here; the bundle audit (scripts/audit-bundle.mjs) is what keeps
// server code out.
export {};
