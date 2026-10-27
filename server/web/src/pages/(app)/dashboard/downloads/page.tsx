// Not upstream: /dashboard/downloads, Downloads (PL "Do pobrania"), the desktop
// app's builds for macOS and Windows (server/docs/SKELETON.md section 9,
// item 45; CONTRACT.md 6.28).
//
// Server (Rust, src/pages/dashboard_downloads.rs): requireUser(), then the
// same object GET /api/downloads answers that account: builds per platform,
// newest first, `keep`, `admin` and, for an administrator, `publishToken`.
// `metadata = { title: "Downloads" }` (translated).
import { DownloadsScreen } from "../../../../downloads/DownloadsScreen";
import type { DownloadsList } from "../../../../downloads/api";

export type DownloadsPageProps = DownloadsList;

export default function DownloadsPage({ props }: { props: DownloadsPageProps }) {
  return <DownloadsScreen list={props} />;
}
