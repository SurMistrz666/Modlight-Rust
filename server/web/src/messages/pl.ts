// The messages app/layout.tsx hands to NextIntlClientProvider for "pl": the
// namespaces i18n/request.ts loads (messages/pl/mail.json is server-only).
// One chunk per locale; src/runtime/messages.ts loads the one in use.
import commonMessages from "@/messages/pl/common.json";
import navMessages from "@/messages/pl/nav.json";
import authMessages from "@/messages/pl/auth.json";
import dashboardMessages from "@/messages/pl/dashboard.json";
import profileMessages from "@/messages/pl/profile.json";
import pasteMessages from "@/messages/pl/paste.json";
import linksMessages from "@/messages/pl/links.json";
import albumsMessages from "@/messages/pl/albums.json";
import tokensMessages from "@/messages/pl/tokens.json";
import adminMessages from "@/messages/pl/admin.json";
import publicMessages from "@/messages/pl/public.json";
import landingMessages from "@/messages/pl/landing.json";
import viewerMessages from "@/messages/pl/viewer.json";
import filesMessages from "@/messages/pl/files.json";
import uploadMessages from "@/messages/pl/upload.json";
import downloadMessages from "@/messages/pl/download.json";
import authorizeMessages from "@/messages/pl/authorize.json";

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
