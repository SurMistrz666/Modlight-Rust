/**
 * Error boundaries with Next's semantics (client/components/error-boundary.js,
 * http-access-fallback, redirect-boundary):
 *
 *  - an error thrown while rendering a segment shows that segment's error.tsx
 *    with `error` and `reset`; the boundary forgets the error when the pathname
 *    changes or new page data arrives (`resetKey`);
 *  - `notFound()` and `redirect()` are not errors to a segment boundary: they
 *    pass through to the root, which renders not-found.tsx or navigates.
 */
import * as React from "react";
import { isNotFoundError, isRedirectError, redirectTarget } from "../runtime/errors";
import { navigate, refresh } from "../runtime/navigation";

export interface ErrorComponentProps {
  error: Error & { digest?: string };
  reset: () => void;
}

interface Props {
  errorComponent: React.ComponentType<ErrorComponentProps>;
  /** The root boundary also handles notFound()/redirect(). */
  root?: boolean;
  notFound?: React.ReactNode;
  resetKey: unknown;
  children: React.ReactNode;
}

interface State {
  error: unknown;
  resetKey: unknown;
}

export class SegmentErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    if (props.resetKey !== state.resetKey) return { error: null, resetKey: props.resetKey };
    return null;
  }

  componentDidCatch(error: unknown): void {
    if (this.props.root && isRedirectError(error) && !error.handled) {
      const { url, type } = redirectTarget(error);
      navigate(url, { replace: type === "replace" });
    }
  }

  reset = (): void => {
    this.setState({ error: null });
    // Next's reset re-renders the segment; the page data here is the server's
    // render, so render it afresh.
    void refresh();
  };

  render(): React.ReactNode {
    const { error } = this.state;
    if (error === null) return this.props.children;
    if (isRedirectError(error)) {
      if (this.props.root) return null;
      throw error;
    }
    if (isNotFoundError(error)) {
      if (this.props.root) return this.props.notFound ?? null;
      throw error;
    }
    const ErrorComponent = this.props.errorComponent;
    const err = error instanceof Error ? error : new Error(String(error));
    return <ErrorComponent error={err as Error & { digest?: string }} reset={this.reset} />;
  }
}

/** Throws during render so the nearest boundary handles it. */
export function Throw({ error }: { error: unknown }): never {
  throw error;
}
