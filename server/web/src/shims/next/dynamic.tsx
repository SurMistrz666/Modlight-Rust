/**
 * `next/dynamic` for the browser build: React.lazy behind a Suspense boundary
 * whose fallback is the `loading` component, which is what Next renders on the
 * client for `{ ssr: false }` (the only way upstream uses it).
 */
import * as React from "react";

export interface DynamicOptionsLoadingProps {
  error?: Error | null;
  isLoading?: boolean;
  pastDelay?: boolean;
  retry?: () => void;
  timedOut?: boolean;
}

export interface DynamicOptions {
  ssr?: boolean;
  loading?: (props: DynamicOptionsLoadingProps) => React.ReactNode;
}

type Loaded<P> = React.ComponentType<P> | { default: React.ComponentType<P> };
type Loader<P> = () => Promise<Loaded<P>>;

export default function dynamic<P extends object = object>(
  loader: Loader<P> | Promise<Loaded<P>>,
  options: DynamicOptions = {},
): React.ComponentType<P> {
  const load = typeof loader === "function" ? loader : () => loader;
  const Lazy = React.lazy(async () => {
    const mod = await load();
    const component = (mod && typeof mod === "object" && "default" in mod ? mod.default : mod) as React.ComponentType<P>;
    return { default: component };
  });
  const Loading = options.loading;

  function DynamicComponent(props: P) {
    const fallback = Loading ? <Loading isLoading pastDelay error={null} /> : null;
    return (
      <React.Suspense fallback={fallback}>
        <Lazy {...(props as P & React.JSX.IntrinsicAttributes)} />
      </React.Suspense>
    );
  }
  return DynamicComponent;
}
