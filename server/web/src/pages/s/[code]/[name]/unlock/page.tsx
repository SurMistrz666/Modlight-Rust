// Port of upstream app/s/[code]/[name]/unlock/page.tsx (namespaced short link gate).
// @upstream app/s/[code]/[name]/unlock/page.tsx
//
// Server (Rust), as upstream: the stored code is scopedShortCode(tag, name),
// then exactly the /s/{code}/unlock page for it (same payload, same metadata).
import ShortlinkUnlockPage, { type ShortlinkUnlockPageProps } from "../../unlock/page";

export type NamespacedShortlinkUnlockPageProps = ShortlinkUnlockPageProps;

export default function NamespacedShortlinkUnlockPage({ props }: { props: NamespacedShortlinkUnlockPageProps }) {
  return <ShortlinkUnlockPage props={props} />;
}
