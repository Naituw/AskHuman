import assert from "node:assert/strict";
import { cp, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { buildSite, landingBody, readTestFlightLink, TESTFLIGHT_FILE } from "./build-ios-beta-site.mjs";

const repo = fileURLToPath(new URL("../", import.meta.url));
const LINK = "https://testflight.apple.com/join/AbCd1234";

async function inTemp(run) {
  const dir = await mkdtemp(resolve(tmpdir(), "ios-beta-site-"));
  try {
    return await run(dir);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

/** A root that holds the repository's real public pages and, when given, a link file. */
async function siteRoot(dir, linkFile) {
  await mkdir(resolve(dir, "root/docs/public"), { recursive: true });
  await cp(resolve(repo, "docs/public/ios"), resolve(dir, "root/docs/public/ios"), { recursive: true });
  await rm(resolve(dir, "root", TESTFLIGHT_FILE), { force: true });
  if (linkFile !== undefined) await writeFile(resolve(dir, "root", TESTFLIGHT_FILE), linkFile);
  return resolve(dir, "root");
}

test("without a link file there is no link", () =>
  inTemp(async (dir) => {
    assert.equal(await readTestFlightLink(resolve(dir, "testflight.json")), null);
  }));

test("a TestFlight join link is accepted", () =>
  inTemp(async (dir) => {
    const file = resolve(dir, "testflight.json");
    await writeFile(file, JSON.stringify({ url: LINK }));
    assert.equal(await readTestFlightLink(file), LINK);
  }));

test("anything else in the link file fails the build", () =>
  inTemp(async (dir) => {
    const file = resolve(dir, "testflight.json");
    const bad = [
      "not json",
      "{}",
      JSON.stringify({ url: 42 }),
      JSON.stringify({ url: "" }),
      JSON.stringify({ url: "http://testflight.apple.com/join/AbCd1234" }),
      JSON.stringify({ url: "https://example.com/join/AbCd1234" }),
      JSON.stringify({ url: "https://testflight.apple.com.evil.example/join/AbCd1234" }),
      JSON.stringify({ url: "https://testflight.apple.com/join/" }),
      JSON.stringify({ url: "https://testflight.apple.com/join/AbCd1234?x=1" }),
      JSON.stringify({ url: "https://testflight.apple.com/join/AbCd1234/extra" }),
      JSON.stringify({ url: `${LINK}"><script>` }),
    ];
    for (const text of bad) {
      await writeFile(file, text);
      await assert.rejects(readTestFlightLink(file), /testflight|JSON/i, text);
    }
  }));

test("a directory where the file should be is an error, not a missing link", () =>
  inTemp(async (dir) => {
    await mkdir(resolve(dir, "testflight.json"));
    await assert.rejects(readTestFlightLink(resolve(dir, "testflight.json")));
  }));

test("the landing page without a link only says the beta is closed", () => {
  const body = landingBody(null);
  assert.match(body, /invitation-only/);
  assert.match(body, /目前仅限邀请/);
  assert.doesNotMatch(body, /class="button"/);
  assert.doesNotMatch(body, /testflight\.apple\.com\/join/);
  assert.doesNotMatch(body, /<li>/);
});

test("the landing page with a link has the button and the three steps in both languages", () => {
  const body = landingBody(LINK);
  assert.equal(body.split(`href="${LINK}"`).length - 1, 2, "one button per language");
  assert.equal(body.split("<li>").length - 1, 6, "three steps per language");
  assert.doesNotMatch(body, /invitation-only|目前仅限邀请/);
  assert.match(body, /<section lang="en">[\s\S]*Join the beta in TestFlight[\s\S]*<section lang="zh-Hans">[\s\S]*在 TestFlight 中加入测试/);
  assert.match(body, /iOS 17/);
  assert.match(body, /Settings → Channels → iOS App/);
  assert.match(body, /设置 → 通信渠道 → iOS App/);
  assert.doesNotMatch(body, /<script/i);
});

test("every page links to the setup guide and the privacy policy", () => {
  for (const link of [null, LINK]) {
    const body = landingBody(link);
    assert.match(body, /ios-setup\.en\.md/);
    assert.match(body, /ios-setup\.md/);
    assert.match(body, /href="privacy\/"/);
    assert.match(body, /href="privacy-zh\/"/);
  }
});

/** Every internal link of every built page leads to a page that was built. */
async function assertInternalLinksResolve(output) {
  const pages = ["", "beta", "beta-zh", "privacy", "privacy-zh"];
  for (const page of pages) {
    const file = resolve(output, page, "index.html");
    const text = await readFile(file, "utf8");
    for (const [, target] of text.matchAll(/href="([^"]+)"/g)) {
      if (/^(https?:|mailto:|#)/.test(target)) continue;
      const resolved = resolve(dirname(file), target);
      const index = target.endsWith("/") || target === "." || target === "./" ? resolve(resolved, "index.html") : resolved;
      await stat(index).catch(() => assert.fail(`${page || "/"}: ${target} leads nowhere`));
    }
  }
}

test("the site builds without a link, as it is published today", () =>
  inTemp(async (dir) => {
    const root = await siteRoot(dir);
    const output = resolve(dir, "out");
    const built = await buildSite({ root, output });
    assert.deepEqual(built, { pages: 5, link: null });
    await stat(resolve(output, ".nojekyll"));
    const landing = await readFile(resolve(output, "index.html"), "utf8");
    assert.match(landing, /invitation-only/);
    assert.doesNotMatch(landing, /testflight\.apple\.com\/join/);
    await assertInternalLinksResolve(output);
  }));

test("the site builds with a link", () =>
  inTemp(async (dir) => {
    const root = await siteRoot(dir, JSON.stringify({ url: LINK }));
    const output = resolve(dir, "out");
    assert.deepEqual(await buildSite({ root, output }), { pages: 5, link: LINK });
    const landing = await readFile(resolve(output, "index.html"), "utf8");
    assert.match(landing, new RegExp(`href="${LINK}"`));
    await assertInternalLinksResolve(output);
  }));

test("a bad link file stops the build before anything is published", () =>
  inTemp(async (dir) => {
    const root = await siteRoot(dir, JSON.stringify({ url: "https://example.com/join/abc" }));
    const output = resolve(dir, "out");
    await assert.rejects(buildSite({ root, output }), /testflight/i);
    await assert.rejects(stat(output), "nothing is written");
  }));
