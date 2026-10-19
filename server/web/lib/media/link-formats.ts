export interface LinkFormats {
  direct: string;
  raw: string;
  markdown: string;
  bbcode: string;
  html: string;
}

/**
 * Build the one-click copy formats for a media. `isImage` decides whether
 * Markdown/BBCode/HTML use image syntax (pointing at the raw URL) or plain links.
 */
export function buildLinkFormats(opts: {
  url: string;
  rawUrl: string;
  filename: string;
  isImage: boolean;
}): LinkFormats {
  const { url, rawUrl, filename, isImage } = opts;
  return {
    direct: url,
    raw: rawUrl,
    markdown: isImage ? `![${filename}](${rawUrl})` : `[${filename}](${url})`,
    bbcode: isImage ? `[img]${rawUrl}[/img]` : `[url]${url}[/url]`,
    html: isImage
      ? `<img src="${rawUrl}" alt="${filename}" />`
      : `<a href="${url}">${filename}</a>`,
  };
}

export const LINK_FORMAT_LABELS: Record<keyof LinkFormats, string> = {
  direct: "Direct link",
  raw: "Raw link",
  markdown: "Markdown",
  bbcode: "BBCode",
  html: "HTML",
};
