"use client";

import { useState } from "react";
import {
  KeyRound,
  Download,
  MousePointerClick,
  Camera,
  Copy,
  Check,
  Terminal,
  UploadCloud,
  ArrowRight,
} from "lucide-react";
import { toast } from "sonner";
import { useTranslations } from "next-intl";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

interface SetupGuideProps {
  appUrl: string;
  appName: string;
}

const STEP_ICONS = [KeyRound, Download, MousePointerClick, Camera] as const;

function CodeBlock({
  label,
  code,
}: {
  label: string;
  code: string;
}) {
  const t = useTranslations("tokens");
  const tc = useTranslations("common");
  const [copied, setCopied] = useState(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      toast.success(t("copiedToClipboard"));
      setTimeout(() => setCopied(false), 1600);
    } catch {
      toast.error(t("clipboardError"));
    }
  }

  return (
    <div className="bg-muted/40 ring-foreground/10 overflow-hidden rounded-lg ring-1">
      <div className="border-border/60 flex items-center justify-between border-b px-3 py-1.5">
        <span className="text-muted-foreground flex items-center gap-1.5 text-[11px] font-medium tracking-wide uppercase">
          <Terminal className="size-3" />
          {label}
        </span>
        <Button type="button" variant="ghost" size="icon-sm" onClick={copy} aria-label={tc("copy")}>
          {copied ? <Check className="text-emerald-500" /> : <Copy />}
        </Button>
      </div>
      <pre className="overflow-x-auto px-3 py-2.5 font-mono text-[11px] leading-relaxed whitespace-pre">
        {code}
      </pre>
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-2">
      <span className="text-muted-foreground text-xs">{label}</span>
      <code className="bg-muted truncate rounded px-1.5 py-0.5 font-mono text-[11px]">
        {value}
      </code>
    </div>
  );
}

export function SetupGuide({ appUrl, appName }: SetupGuideProps) {
  const t = useTranslations("tokens");
  const curl = [
    `curl -H "x-api-key: YOUR_TOKEN" \\`,
    `  -F "upload=@screenshot.png" \\`,
    `  ${appUrl}/upload`,
  ].join("\n");

  const oneLiner = `curl -s -H "x-api-key: YOUR_TOKEN" -F "upload=@$1" ${appUrl}/upload | grep -o '"url":"[^"]*"' | cut -d'"' -f4`;

  const steps = STEP_ICONS.map((icon, i) => ({
    icon,
    title: t(`guide.step${i + 1}Title`),
    body: t(`guide.step${i + 1}Body`),
  }));

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <UploadCloud className="text-primary size-4" />
            {t("guide.setupTitle")}
          </CardTitle>
          <CardDescription>
            {t("guide.setupDesc", { appName })}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <ol className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {steps.map((step, i) => (
              <li
                key={step.title}
                className="border-border/60 bg-muted/30 relative flex flex-col gap-2.5 rounded-xl border p-4"
              >
                <div className="flex items-center justify-between">
                  <div className="bg-primary/10 text-primary grid size-9 place-items-center rounded-lg">
                    <step.icon className="size-4" />
                  </div>
                  <span className="text-muted-foreground/40 font-heading text-2xl font-semibold tabular-nums">
                    {i + 1}
                  </span>
                </div>
                <div className="space-y-1">
                  <p className="text-sm font-medium">{step.title}</p>
                  <p className="text-muted-foreground text-xs leading-relaxed">
                    {step.body}
                  </p>
                </div>
                {i < steps.length - 1 && (
                  <ArrowRight className="text-border absolute top-1/2 -right-3 hidden size-4 -translate-y-1/2 lg:block" />
                )}
              </li>
            ))}
          </ol>

          <div className="text-muted-foreground bg-muted/30 ring-foreground/10 mt-4 flex items-start gap-2.5 rounded-lg p-3 text-xs ring-1">
            <MousePointerClick className="text-primary mt-0.5 size-4 shrink-0" />
            <p className="leading-relaxed">
              {t.rich("guide.noShareX", {
                link: (chunks) => (
                  <a
                    href="/dashboard/upload"
                    className="text-foreground font-medium underline underline-offset-2"
                  >
                    {chunks}
                  </a>
                ),
              })}
            </p>
          </div>
        </CardContent>
      </Card>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{t("guide.rawTitle")}</CardTitle>
            <CardDescription>
              {t("guide.rawDesc")}
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="divide-border/60 divide-y">
              <Field label={t("guide.fieldMethod")} value="POST" />
              <Field label={t("guide.fieldUrl")} value={`${appUrl}/upload`} />
              <Field label={t("guide.fieldAuthHeader")} value="x-api-key: <token>" />
              <Field label={t("guide.fieldFieldName")} value={t("guide.fieldFieldNameValue")} />
              <Field label={t("guide.fieldOptional")} value={t("guide.fieldOptionalValue")} />
            </div>
            <p className="text-muted-foreground mt-3 text-xs leading-relaxed">
              {t("guide.respondsWith")}{" "}
              <code className="bg-muted rounded px-1 py-0.5 font-mono text-[11px]">
                url
              </code>
              ,{" "}
              <code className="bg-muted rounded px-1 py-0.5 font-mono text-[11px]">
                raw_url
              </code>
              ,{" "}
              <code className="bg-muted rounded px-1 py-0.5 font-mono text-[11px]">
                deletionUrl
              </code>
              .
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-sm">{t("guide.terminalTitle")}</CardTitle>
            <CardDescription>
              {t.rich("guide.terminalDesc", {
                code: (chunks) => (
                  <code className="bg-muted rounded px-1 py-0.5 font-mono text-[11px]">
                    {chunks}
                  </code>
                ),
              })}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <CodeBlock label="curl" code={curl} />
            <CodeBlock label={t("guide.oneLinerLabel")} code={oneLiner} />
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
