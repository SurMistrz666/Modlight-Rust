// Port of upstream app/page.tsx.
// @upstream app/page.tsx
//
// Upstream never renders anything here: getSession(), then
// redirect(session ? "/dashboard" : "/login"). The server answers this URL with
// that redirect (HTTP 307 on a document request, a redirect payload on /_page),
// so this component only exists to complete the route table.

/** No props: the page always redirects. */
export type RootPageProps = Record<string, never>;

export default function Root(_: { props: RootPageProps }) {
  return null;
}
