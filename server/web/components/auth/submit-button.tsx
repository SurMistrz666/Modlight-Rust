"use client";

import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * Primary form action that owns its own loading affordance: a centered spinner
 * + label swap, full width, sized up for the auth forms (the base Button is
 * intentionally compact for dense dashboard UIs).
 */
export function SubmitButton({
  pending,
  children,
  pendingText,
  className,
  ...props
}: React.ComponentProps<typeof Button> & {
  pending?: boolean;
  pendingText?: string;
}) {
  return (
    <Button
      type="submit"
      disabled={pending || props.disabled}
      aria-busy={pending}
      className={cn("h-10 w-full text-sm", className)}
      {...props}
    >
      {pending ? (
        <>
          <Loader2 className="size-4 animate-spin" />
          {pendingText ?? children}
        </>
      ) : (
        children
      )}
    </Button>
  );
}
