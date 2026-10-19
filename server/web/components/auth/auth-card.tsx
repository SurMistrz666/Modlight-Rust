import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * The single elegant card that frames every auth screen. Slightly heavier ring,
 * inner highlight and soft backdrop blur to lift it off the ambient gradient in
 * the auth layout.
 */
export function AuthCard({
  title,
  description,
  children,
  footer,
  className,
}: {
  title: React.ReactNode;
  description?: React.ReactNode;
  children: React.ReactNode;
  footer?: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "bg-card/80 supports-[backdrop-filter]:bg-card/60 relative overflow-hidden rounded-2xl p-6 ring-1 ring-foreground/10 backdrop-blur-xl sm:p-8",
        "shadow-[0_1px_0_0_rgba(255,255,255,0.04)_inset,0_20px_60px_-20px_rgba(0,0,0,0.45)]",
        className,
      )}
    >
      {/* Subtle violet top edge to anchor the brand accent. */}
      <div
        aria-hidden
        className="pointer-events-none absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-primary/60 to-transparent"
      />
      <div className="mb-6 space-y-1.5">
        <h1 className="font-heading text-xl font-semibold tracking-tight">
          {title}
        </h1>
        {description ? (
          <p className="text-muted-foreground text-sm leading-relaxed text-balance">
            {description}
          </p>
        ) : null}
      </div>
      {children}
      {footer ? (
        <div className="text-muted-foreground mt-6 border-t border-border/60 pt-5 text-center text-sm">
          {footer}
        </div>
      ) : null}
    </div>
  );
}
