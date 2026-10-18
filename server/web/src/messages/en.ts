// The messages app/layout.tsx hands to NextIntlClientProvider for "en": the
// namespaces i18n/request.ts loads (messages/en/mail.json is server-only).
// One chunk per locale; src/runtime/messages.ts loads the one in use.
import commonMessages from "@/messages/en/common.json";
import navMessages from "@/messages/en/nav.json";
import authMessages from "@/messages/en/auth.json";
import dashboardMessages from "@/messages/en/dashboard.json";
import profileMessages from "@/messages/en/profile.json";
import pasteMessages from "@/messages/en/paste.json";
import linksMessages from "@/messages/en/links.json";
import albumsMessages from "@/messages/en/albums.json";
import tokensMessages from "@/messages/en/tokens.json";
import adminMessages from "@/messages/en/admin.json";
import publicMessages from "@/messages/en/public.json";
import landingMessages from "@/messages/en/landing.json";
import viewerMessages from "@/messages/en/viewer.json";
import filesMessages from "@/messages/en/files.json";
import uploadMessages from "@/messages/en/upload.json";
import downloadMessages from "@/messages/en/download.json";
import authorizeMessages from "@/messages/en/authorize.json";

export default {
  common: commonMessages,
  nav: navMessages,
  auth: authMessages,
  dashboard: dashboardMessages,
  profile: profileMessages,
  paste: pasteMessages,
  links: linksMessages,
  albums: albumsMessages,
  tokens: tokensMessages,
  admin: adminMessages,
  public: publicMessages,
  landing: landingMessages,
  viewer: viewerMessages,
  files: filesMessages,
  upload: uploadMessages,
  download: downloadMessages,
  authorize: authorizeMessages,
};
