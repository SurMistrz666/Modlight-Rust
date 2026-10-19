"use client";

import { useEffect, useState, useTransition } from "react";
import { useTranslations } from "next-intl";
import { toast } from "sonner";
import { PackageCheck, PackageX, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { humanFileSize } from "@/lib/format";
import {
  loadRelease,
  publishAppRelease,
  unpublishAppRelease,
  type ReleaseState,
} from "@/app/actions/releases";

/**
 * The desktop app's download, pointed at a file.
 *
 * A release here is one settings row naming an ordinary upload - which is why
 * this form asks for a link rather than offering its own uploader. The installer
 * goes up the same way everything else does (drag it onto the dashboard), and
 * that buys streaming, resume, Range, checksums and the storage driver for free.
 */
export function ReleaseChannel() {
  const t = useTranslations("admin");
  const [state, setState] = useState<ReleaseState | null>(null);
  const [version, setVersion] = useState("");
  const [code, setCode] = useState("");
  const [notes, setNotes] = useState("");
  const [pending, startTransition] = useTransition();
  // Set when the server refused because the installer's file name names a
  // different version. Holds what the file says, so the second click is an
  // informed one rather than the same click repeated harder.
  const [mismatch, setMismatch] = useState<{ fileVersion: string; filename: string } | null>(null);

  useEffect(() => {
    loadRelease()
      .then(setState)
      .catch(() => setState(null));
  }, []);

  function refresh() {
    loadRelease().then(setState).catch(() => undefined);
  }

  function onPublish(force = false) {
    startTransition(async () => {
      const res = await publishAppRelease({ version, code, notes, allowVersionMismatch: force });
      if (res.ok) {
        toast.success(t("release.published", { version: res.manifest.version }));
        setCode("");
        setNotes("");
        setMismatch(null);
        refresh();
        return;
      }

      // A version the file itself contradicts is not a refusal to argue with in
      // a toast: it is usually right, and when it is wrong the admin knows why.
      // So the button turns into "publish anyway" and nothing is written until
      // it is pressed a second time.
      if (res.reason === "version-mismatch") {
        setMismatch({ fileVersion: res.fileVersion, filename: res.filename });
        toast.error(
          t("release.errors.version-mismatch", {
            typed: version.trim(),
            fileVersion: res.fileVersion,
            filename: res.filename,
          }),
        );
        return;
      }

      // Each refusal names the thing to go and change, because "failed" would
      // leave an admin guessing between four quite different problems - and one
      // of them (a default file password stamped on the upload) is invisible
      // from the file's own page.
      setMismatch(null);
      toast.error(t(`release.errors.${res.reason}`));
    });
  }

  function onUnpublish() {
    startTransition(async () => {
      await unpublishAppRelease();
      toast.success(t("release.unpublished"));
      refresh();
    });
  }

  const manifest = state?.manifest ?? null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <PackageCheck className="size-4" />
          {t("release.title", { app: state?.appName ?? "" })}
        </CardTitle>
        <CardDescription>{t("release.desc")}</CardDescription>
      </CardHeader>

      <CardContent className="space-y-5">
        <div className="bg-muted/40 rounded-[var(--radius)] border p-4 text-sm">
          {manifest ? (
            <div className="space-y-1">
              <p className="font-medium">
                {t("release.current", { version: manifest.version })}
                {" · "}
                <span className="text-muted-foreground tabular-nums">
                  {humanFileSize(manifest.size)}
                </span>
              </p>
              <p className="text-muted-foreground break-all text-xs">{manifest.url}</p>
            </div>
          ) : (
            <p className="text-muted-foreground">{t("release.none")}</p>
          )}
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label htmlFor="release-version">{t("release.versionLabel")}</Label>
            <Input
              id="release-version"
              value={version}
              onChange={(e) => {
                setVersion(e.target.value);
                setMismatch(null);
              }}
              placeholder="0.7.0"
              disabled={pending}
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="release-code">{t("release.codeLabel")}</Label>
            <Input
              id="release-code"
              value={code}
              onChange={(e) => {
                setCode(e.target.value);
                setMismatch(null);
              }}
              placeholder="https://host.example.com/tag/abcde12345.exe"
              disabled={pending}
            />
          </div>
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="release-notes">{t("release.notesLabel")}</Label>
          <Input
            id="release-notes"
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            disabled={pending}
          />
        </div>

        <p className="text-muted-foreground text-xs leading-relaxed">{t("release.hint")}</p>

        {mismatch ? (
          <p className="text-destructive text-sm leading-relaxed">
            {t("release.errors.version-mismatch", {
              typed: version.trim(),
              fileVersion: mismatch.fileVersion,
              filename: mismatch.filename,
            })}
          </p>
        ) : null}

        <div className="flex flex-wrap gap-2">
          <Button
            onClick={() => onPublish(mismatch !== null)}
            disabled={pending || !version.trim() || !code.trim()}
            variant={mismatch ? "destructive" : "default"}
          >
            <PackageCheck className="size-4" />
            {mismatch ? t("release.publishAnyway") : t("release.publish")}
          </Button>
          <Button variant="outline" onClick={refresh} disabled={pending}>
            <RefreshCw className="size-4" />
            {t("release.refresh")}
          </Button>
          {manifest ? (
            <Button variant="ghost" onClick={onUnpublish} disabled={pending}>
              <PackageX className="size-4" />
              {t("release.unpublish")}
            </Button>
          ) : null}
        </div>
      </CardContent>
    </Card>
  );
}
