// Port of upstream app/layout.tsx (the root layout).
// @upstream app/layout.tsx
//
// Upstream renders <html> and <body> itself; here those belong to the HTML
// document the Rust server sends (index.html with the attributes filled in,
// see docs/CONTRACT.md "HTML shell"): lang={locale}, data-mode={mode},
// data-theme={skin}, the two Geist font classes plus "h-full antialiased", and
// <body class="bg-background text-foreground min-h-full">. Later changes of
// those values are applied by src/routes/root.tsx the way React would update
// the rendered attributes. `metadata` (title template, description,
// applicationName) is part of every page's `meta` in the payload.
//
// What stays is the provider stack, prop for prop.
import { NextIntlClientProvider } from "next-intl";
import { ThemeProvider } from "@/components/theme-provider";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";
import type { RootLayoutData } from "../contract/layouts";
import type { Messages } from "../runtime/messages";

export default function RootLayout({
  data,
  messages,
  children,
}: Readonly<{ data: RootLayoutData; messages: Messages; children: React.ReactNode }>) {
  const { locale, mode, timeZone } = data;
  return (
    // NextIntlClientProvider rendered from a Server Component inherits timeZone
    // (and formats/now, unset upstream) from the request config: passed explicitly.
    <NextIntlClientProvider locale={locale} messages={messages} timeZone={timeZone}>
      <ThemeProvider
        attribute="data-mode"
        // Seeded from the cookie so a cleared localStorage still lands on
        // the mode the server just rendered.
        defaultTheme={mode}
        enableSystem
        disableTransitionOnChange
      >
        <TooltipProvider delayDuration={200}>{children}</TooltipProvider>
        <Toaster richColors position="bottom-right" />
      </ThemeProvider>
    </NextIntlClientProvider>
  );
}
