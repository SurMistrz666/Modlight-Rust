"use client";

import * as React from "react";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface CopyButtonProps extends React.ComponentProps<typeof Button> {
  value: string;
  label?: string;
  toastMessage?: string;
}

/** A button that copies `value` to the clipboard with a transient check state. */
export function CopyButton({
  value,
  label,
  toastMessage,
  children,
  className,
  variant = "outline",
  size = "sm",
  ...props
}: CopyButtonProps) {
  const t = useTranslations("viewer");
  const tCommon = useTranslations("common");
  const [copied, setCopied] = React.useState(false);
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);

  React.useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current);
  }, []);

  async function onCopy() {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      toast.success(toastMessage ?? `${tCommon("copied")}`);
      if (timer.current) clearTimeout(timer.current);
      timer.current = setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("copyButton.copyFailedBlocked"));
    }
  }

  return (
    <Button
      type="button"
      variant={variant}
      size={size}
      onClick={onCopy}
      className={cn(className)}
      {...props}
    >
      {copied ? (
        <Check className="text-emerald-500" />
      ) : (
        children ?? <Copy />
      )}
      {label ? <span>{copied ? tCommon("copied") : label}</span> : null}
    </Button>
  );
}
