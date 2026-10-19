"use client";

import * as React from "react";
import { Eye, EyeOff } from "lucide-react";
import { useTranslations } from "next-intl";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/**
 * Password field with an inline show/hide toggle. Forwards every native input
 * prop (so it composes with required/autoComplete/aria-invalid, etc.) and only
 * fixes the trailing padding to clear the toggle button.
 */
export function PasswordInput({
  className,
  id,
  ...props
}: Omit<React.ComponentProps<typeof Input>, "type">) {
  const t = useTranslations("auth");
  const [visible, setVisible] = React.useState(false);

  return (
    <div className="relative">
      <Input
        id={id}
        type={visible ? "text" : "password"}
        className={cn("h-10 pr-10", className)}
        {...props}
      />
      <button
        type="button"
        onClick={() => setVisible((v) => !v)}
        tabIndex={-1}
        aria-label={visible ? t("hidePassword") : t("showPassword")}
        className="text-muted-foreground hover:text-foreground absolute inset-y-0 right-0 grid w-10 place-items-center rounded-r-lg outline-none transition-colors focus-visible:text-foreground"
      >
        {visible ? <EyeOff className="size-4" /> : <Eye className="size-4" />}
      </button>
    </div>
  );
}
