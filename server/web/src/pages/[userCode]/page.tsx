// Port of upstream app/[userCode]/page.tsx (tagless media links: /{code}.{ext}).
// @upstream app/[userCode]/page.tsx
//
// Server (Rust), as upstream: resolveTag(segment) - not reserved
// (isReservedTag), owner opted into tagless links (resolveTaglessOwnerTag) -
// else notFound; then exactly the /{tag}/{code} page with the resolved tag, so
// the payload is a MediaPageProps. generateMetadata: the media page's, or
// { title: "Not found", robots: { index: false, follow: false } }.
import MediaPage, { type MediaPageProps } from "./[mediaCode]/page";

export type TaglessMediaPageProps = MediaPageProps;

export default function TaglessMediaPage({ props }: { props: TaglessMediaPageProps }) {
  return <MediaPage props={props} />;
}
