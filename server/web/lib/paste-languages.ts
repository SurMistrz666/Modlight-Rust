// Plain module (NOT "use server") so the language list can be imported by both
// the server action and client components. A "use server" file may only export
// async server actions, so constants must live here.

export const LANG_EXT: Record<string, string> = {
  text: "txt", ts: "ts", tsx: "tsx", js: "js", jsx: "jsx", json: "json",
  py: "py", go: "go", rs: "rs", java: "java", c: "c", cpp: "cpp", cs: "cs",
  php: "php", rb: "rb", sh: "sh", yml: "yml", toml: "toml", sql: "sql",
  html: "html", css: "css", md: "md", xml: "xml", kt: "kt", swift: "swift",
  dockerfile: "dockerfile", ini: "ini", diff: "diff",
};

export const PASTE_LANGUAGES: string[] = Object.keys(LANG_EXT);
