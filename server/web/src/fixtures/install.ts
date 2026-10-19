/**
 * Fixture mode (`vite --mode fixtures`): pages come from fixtures/pages/*.json,
 * server actions are answered from fixtures/actions.json ("<module>.<export>":
 * envelope) or with an error envelope naming the missing answer.
 */
import { setPageSource } from "../routes/loader";
import { setActionTransport } from "../runtime/rpc";
import { routes } from "../routes/tree";
import { decodeTagged } from "../runtime/tagged-json";
import type { PageNotFound } from "../runtime/payload";
import { MASKED_SERVER_ERROR } from "../runtime/errors";
import { expandFixture, fixtureFor } from "./source";
import actions from "../../fixtures/actions.json";

export async function installFixtures(): Promise<void> {
  setPageSource(async (url) => {
    const file = fixtureFor(routes, url);
    if (!file) {
      const { layouts } = expandFixture({ url: "", payload: { layouts: "$root" } }) as PageNotFound;
      return { notFound: true, layouts, meta: { title: "Modlight", robots: "noindex" } } satisfies PageNotFound;
    }
    return expandFixture(file);
  });
  setActionTransport(async (module, name, args) => {
    const key = `${module}.${name}`;
    const answer = (actions as Record<string, unknown>)[key];
    console.info(`[fixtures] ${key}`, args);
    if (answer === undefined) {
      return { error: { message: MASKED_SERVER_ERROR, digest: `fixture-missing:${key}` } };
    }
    return decodeTagged(answer);
  });
}
