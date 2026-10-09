import {test} from "node:test";
import assert from "node:assert/strict";
import {manifest} from "./release-manifest.mjs";
import {distributionPlan, publishFeed, readFeed} from "./publish-update-feed.mjs";

const update = version => manifest(version, `v${version}`, `Discord to VR_${version}_x64-setup.exe`,
  Buffer.from(`trusted comment: version:${version}\n`).toString("base64"));
const snapshot = (version = "0.2.2") => ({head: "old-head", tree: "old-tree", bridgeVersion: "0.2.2", version});

function githubFixture(state, version, includeLegacyManifest, overrides = {}) {
  const calls = [];
  const encoded = value => ({encoding: "base64", content: Buffer.from(JSON.stringify(value)).toString("base64")});
  const request = async (path, {method = "GET", body} = {}) => {
    calls.push({path, method, body});
    if (method === "GET") {
      if (path.endsWith("/git/ref/heads/updates")) return state ? {object: {sha: state.head}} : null;
      if (path.includes("/git/commits/")) return {tree: {sha: state.tree}};
      if (path.includes("/contents/migration.json?ref=")) return encoded({bridgeVersion: state.bridgeVersion});
      if (path.includes("/contents/latest.json?ref=")) return encoded(update(state.version));
      if (path.includes("/releases/tags/")) return {
        draft: false, prerelease: false, assets: [
          {name: `Discord.to.VR_${version}_x64-setup.exe`},
          ...(includeLegacyManifest ? [{name: "latest.json"}] : []),
        ], ...overrides,
      };
    }
    if (path.endsWith("/git/trees")) return {sha: "new-tree"};
    if (path.endsWith("/git/commits")) return {sha: "new-head"};
    if (path.includes("/git/refs")) return {};
    throw new Error(`Unexpected fixture request: ${method} ${path}`);
  };
  return {request, calls, writes: () => calls.filter(call => call.method !== "GET")};
}

test("only the migration release retains latest.json, including retries and minor version jumps", () => {
  const first = distributionPlan("0.2.2", null);
  assert.equal(first.includeLegacyManifest, true);
  assert.deepEqual(first.files, ["target/tauri/release/bundle/nsis/Discord to VR_0.2.2_x64-setup.exe", "latest.json"]);
  assert.equal(distributionPlan("0.2.2", snapshot()).includeLegacyManifest, true);
  assert.equal(distributionPlan("0.2.3", snapshot()).includeLegacyManifest, false);
  assert.equal(distributionPlan("0.2.3", snapshot("0.2.3")).includeLegacyManifest, false);
  assert.deepEqual(distributionPlan("1.0.0", snapshot()).files,
    ["target/tauri/release/bundle/nsis/Discord to VR_1.0.0_x64-setup.exe"]);
  assert.throws(() => distributionPlan("0.2.2", snapshot("0.2.3")));
  assert.throws(() => distributionPlan("0.2.3", {...snapshot(), bridgeVersion: "0.2.4"}));
});

test("the first feed is created only after its signed installer and compatibility manifest are published", async () => {
  const fixture = githubFixture(null, "0.2.2", true);
  assert.equal(await readFeed(fixture.request), null);
  const payload = update("0.2.2");
  await publishFeed(fixture.request, payload, distributionPlan("0.2.2", null));
  const [tree, commit, ref] = fixture.writes();
  assert.deepEqual(tree.body.tree.map(file => file.path), ["latest.json", "migration.json"]);
  assert.deepEqual(JSON.parse(tree.body.tree[0].content), payload);
  assert.deepEqual(JSON.parse(tree.body.tree[1].content), {bridgeVersion: "0.2.2"});
  assert.deepEqual(commit.body.parents, []);
  assert.match(commit.body.message, /\[skip ci\]/);
  assert.deepEqual(ref.body, {ref: "refs/heads/updates", sha: "new-head"});
  assert.ok(fixture.calls.findIndex(call => call.path.includes("/releases/tags/")) <
    fixture.calls.findIndex(call => call.method === "POST"));
});

test("later publications preserve the migration record and existing tree without force pushing", async () => {
  const state = snapshot();
  const fixture = githubFixture(state, "0.2.3", false);
  await publishFeed(fixture.request, update("0.2.3"), distributionPlan("0.2.3", state));
  const [tree, commit, ref] = fixture.writes();
  assert.equal(tree.body.base_tree, "old-tree");
  assert.deepEqual(JSON.parse(tree.body.tree[1].content), {bridgeVersion: "0.2.2"});
  assert.deepEqual(commit.body.parents, ["old-head"]);
  assert.equal(ref.method, "PATCH");
  assert.deepEqual(ref.body, {sha: "new-head", force: false});
});

test("a concurrent feed change or incorrect public release leaves the feed untouched", async () => {
  for (const overrides of [
    {draft: true}, {prerelease: true}, {assets: []},
    {assets: [{name: "Discord.to.VR_0.2.3_x64-setup.exe"}, {name: "latest.json"}]},
  ]) {
    const fixture = githubFixture(snapshot(), "0.2.3", false, overrides);
    await assert.rejects(publishFeed(fixture.request, update("0.2.3"), distributionPlan("0.2.3", snapshot())));
    assert.equal(fixture.writes().length, 0);
  }
  const fixture = githubFixture({...snapshot(), head: "concurrent-head"}, "0.2.3", false);
  await assert.rejects(publishFeed(fixture.request, update("0.2.3"), distributionPlan("0.2.3", snapshot())));
  assert.equal(fixture.writes().length, 0);
});

test("wrong signatures and installer URLs cannot be published to the update feed", async () => {
  for (const payload of [
    {...update("0.2.2"), version: "0.2.3"},
    {...update("0.2.3"), platforms: {"windows-x86_64": {
      ...update("0.2.3").platforms["windows-x86_64"], url: "https://example.com/setup.exe",
    }}},
  ]) {
    const fixture = githubFixture(snapshot(), "0.2.3", false);
    await assert.rejects(publishFeed(fixture.request, payload, distributionPlan("0.2.3", snapshot())));
    assert.equal(fixture.writes().length, 0);
  }
});
