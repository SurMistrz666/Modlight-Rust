/**
 * Settings -> Appearance & security: "Metadata in uploads" (docs/SKELETON.md
 * section 9 item 47). Not upstream.
 *
 * One switch, "Remove metadata from files on upload", what it removes, the file
 * types it cleans, the note that files already uploaded stay as they are, and
 * the files that could not be cleaned (stored as they came) with the reason.
 * The switch moves at once and goes back when the server does not take it.
 */
import * as React from "react";
import Link from "next/link";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { Eraser, Info, TriangleAlert } from "lucide-react";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { buildOpenUrl } from "@/components/dashboard/media";
import { setStripMetadata, type KeptFile, type StripStatus } from "./actions";

const OFF: StripStatus = { enabled: false, kept: [], keptTotal: 0, formats: [] };

/** The dictionary key of a reason word (`too-large` -> `tooLarge`). */
const REASON_KEYS: Record<string, string> = {
  damaged: "damaged",
  format: "format",
  protected: "protected",
  signed: "signed",
  "too-large": "tooLarge",
  busy: "busy",
  tool: "tool",
};

export function StripMetadataSettings({ initial, userCode }: { initial?: StripStatus; userCode: string | null }) {
  const t = useTranslations("stripMetadata.settings");
  const [status, setStatus] = React.useState<StripStatus>(initial ?? OFF);
  const [pending, setPending] = React.useState(false);

  async function toggle(next: boolean) {
    const before = status;
    setStatus({ ...status, enabled: next });
    setPending(true);
    try {
      const saved = await setStripMetadata(next);
      setStatus(saved);
      toast.success(saved.enabled ? t("turnedOn") : t("turnedOff"));
    } catch {
      // The server did not take it: the switch shows what is stored.
      setStatus(before);
      toast.error(t("error"));
    } finally {
      setPending(false);
    }
  }

  return (
    <Card className="xl:col-span-2" data-strip-metadata={status.enabled ? "on" : "off"}>
      <CardHeader className="border-b">
        <CardTitle className="flex items-center gap-2">
          <Eraser className="text-primary size-4" />
          {t("title")}
        </CardTitle>
        <CardDescription>{t("description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-5">
        <div className="flex items-start justify-between gap-4">
          <div className="space-y-1">
            <Label htmlFor="ml-strip-metadata" className="cursor-pointer">
              {t("switch")}
            </Label>
            <p id="ml-strip-metadata-help" className="text-muted-foreground text-sm leading-snug">
              {t("switchHelp")}
            </p>
          </div>
          <Switch
            id="ml-strip-metadata"
            checked={status.enabled}
            onCheckedChange={toggle}
            disabled={pending}
            aria-describedby="ml-strip-metadata-help"
          />
        </div>

        {status.formats.length > 0 && (
          <div className="space-y-2">
            <p className="text-muted-foreground text-xs font-semibold tracking-wider uppercase">{t("formats")}</p>
            <div className="flex flex-wrap gap-1.5" data-strip-formats>
              {status.formats.map((format) => (
                <Badge key={format} variant="outline" className="font-mono text-[11px]">
                  {format}
                </Badge>
              ))}
            </div>
          </div>
        )}

        <ul className="text-muted-foreground space-y-1.5 text-xs">
          <li className="flex items-start gap-2">
            <Info className="mt-px size-3.5 shrink-0" />
            {t("existing")}
          </li>
          <li className="flex items-start gap-2">
            <Info className="mt-px size-3.5 shrink-0" />
            {t("orientation")}
          </li>
          <li className="flex items-start gap-2">
            <Info className="mt-px size-3.5 shrink-0" />
            {t("documents")}
          </li>
        </ul>

        {status.kept.length > 0 && <KeptList status={status} userCode={userCode} />}
      </CardContent>
    </Card>
  );
}

function KeptList({ status, userCode }: { status: StripStatus; userCode: string | null }) {
  const t = useTranslations("stripMetadata.settings");
  return (
    <div className="space-y-2 rounded-md border border-amber-500/30 bg-amber-500/5 p-3" data-strip-kept>
      <p className="flex items-center gap-2 text-sm font-medium">
        <TriangleAlert className="size-4 shrink-0 text-amber-500" />
        {t("kept", { count: status.keptTotal })}
      </p>
      <p className="text-muted-foreground text-xs">
        {t("keptHelp")}
        {status.keptTotal > status.kept.length && <> {t("keptShown", { shown: status.kept.length })}</>}
      </p>
      <ul className="divide-foreground/10 divide-y text-sm">
        {status.kept.map((file) => (
          <KeptRow key={file.id} file={file} userCode={userCode} />
        ))}
      </ul>
    </div>
  );
}

function KeptRow({ file, userCode }: { file: KeptFile; userCode: string | null }) {
  const t = useTranslations("stripMetadata.settings");
  const key = REASON_KEYS[file.reason] ?? "tool";
  return (
    <li className="flex items-center justify-between gap-3 py-1.5" data-kept-reason={file.reason}>
      {userCode ? (
        <Link href={buildOpenUrl(userCode, file.code, file.ext)} className="min-w-0 truncate hover:underline">
          {file.filename}
        </Link>
      ) : (
        <span className="min-w-0 truncate">{file.filename}</span>
      )}
      <span className="text-muted-foreground shrink-0 text-xs">{t(`reasons.${key}`)}</span>
    </li>
  );
}
