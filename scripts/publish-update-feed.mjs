import {appendFile, mkdir, readFile, writeFile} from "node:fs/promises";
import {dirname, join, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {feedBranch, manifest, repository, validateVersion} from "./release-manifest.mjs";

const apiRoot = `/repos/${repository}`;
const installerName = version => `Discord to VR_${version}_x64-setup.exe`;
const publishedInstaller = version => installerName(version).replaceAll(" ", ".");

function compareVersions(left, right) {
  const parse = value => {
    if (!/^\d+\.\d+\.\d+$/.test(value)) throw new Error("Invalid stable version.");
    return value.split(".").map(BigInt);
  };
  const a = parse(left), b = parse(right);
  for (let index = 0; index < a.length; index++) {
    if (a[index] !== b[index]) return a[index] > b[index] ? 1 : -1;
  }
  return 0;
}

export function distributionPlan(version, snapshot) {
  compareVersions(version, version);
  const bridgeVersion = snapshot?.bridgeVersion ?? version;
  if (snapshot && (compareVersions(version, snapshot.version) < 0 ||
      compareVersions(bridgeVersion, snapshot.version) > 0)) {
    throw new Error("Update publication cannot move backwards or reset the migration.");
  }
  const includeLegacyManifest = version === bridgeVersion;
  return {
    version, bridgeVersion, includeLegacyManifest, expectedHead: snapshot?.head ?? null,
    files: [
      `target/tauri/release/bundle/nsis/${installerName(version)}`,
      ...(includeLegacyManifest ? ["latest.json"] : []),
    ],
  };
}

export async function readFeed(request) {
  const ref = await request(`${apiRoot}/git/ref/heads/${feedBranch}`, {allowMissing: true});
  if (!ref) return null;
  const head = ref.object.sha;
  const [commit, migration, latest] = await Promise.all([
    request(`${apiRoot}/git/commits/${head}`),
    request(`${apiRoot}/contents/migration.json?ref=${head}`),
    request(`${apiRoot}/contents/latest.json?ref=${head}`),
  ]);
  const decode = file => {
    if (file.encoding !== "base64" || typeof file.content !== "string") {
      throw new Error("Unexpected update feed contents.");
    }
    return JSON.parse(Buffer.from(file.content, "base64").toString("utf8"));
  };
  const {bridgeVersion} = decode(migration);
  const {version} = decode(latest);
  if (typeof bridgeVersion !== "string" || typeof version !== "string") {
    throw new Error("The existing update feed has no valid migration state.");
  }
  return {head, tree: commit.tree.sha, bridgeVersion, version};
}

export async function publishFeed(request, update, plan) {
  const signature = update.platforms?.["windows-x86_64"]?.signature;
  if (typeof signature !== "string" || update.version !== plan.version) {
    throw new Error("The update manifest does not match the planned release.");
  }
  const expected = manifest(plan.version, `v${plan.version}`, installerName(plan.version), signature);
  if (update.platforms["windows-x86_64"].url !== expected.platforms["windows-x86_64"].url) {
    throw new Error("Unexpected update installer URL.");
  }
  const snapshot = await readFeed(request);
  const currentPlan = distributionPlan(plan.version, snapshot);
  if (currentPlan.expectedHead !== plan.expectedHead ||
      currentPlan.bridgeVersion !== plan.bridgeVersion ||
      currentPlan.includeLegacyManifest !== plan.includeLegacyManifest) {
    throw new Error("The update feed changed during the build; rerun the release workflow.");
  }
  const release = await request(`${apiRoot}/releases/tags/v${plan.version}`);
  const assets = release.assets.map(asset => asset.name).sort();
  const expectedAssets = [publishedInstaller(plan.version),
    ...(plan.includeLegacyManifest ? ["latest.json"] : [])].sort();
  if (release.draft || release.prerelease || JSON.stringify(assets) !== JSON.stringify(expectedAssets)) {
    throw new Error("Publish the planned stable release assets before updating the feed.");
  }
  const json = value => JSON.stringify(value, null, 2) + "\n";
  const tree = await request(`${apiRoot}/git/trees`, {method: "POST", body: {
    ...(snapshot ? {base_tree: snapshot.tree} : {}),
    tree: [
      {path: "latest.json", mode: "100644", type: "blob", content: json(update)},
      {path: "migration.json", mode: "100644", type: "blob", content: json({bridgeVersion: plan.bridgeVersion})},
    ],
  }});
  const commit = await request(`${apiRoot}/git/commits`, {method: "POST", body: {
    message: `chore: publish update feed for v${plan.version} [skip ci]`,
    tree: tree.sha, parents: snapshot ? [snapshot.head] : [],
  }});
  if (snapshot) {
    await request(`${apiRoot}/git/refs/heads/${feedBranch}`, {
      method: "PATCH", body: {sha: commit.sha, force: false},
    });
  } else {
    await request(`${apiRoot}/git/refs`, {
      method: "POST", body: {ref: `refs/heads/${feedBranch}`, sha: commit.sha},
    });
  }
}

async function main() {
  if (process.env.GITHUB_ACTIONS !== "true" || process.env.GITHUB_REPOSITORY !== repository ||
      process.env.GITHUB_REF_TYPE !== "tag" || !process.env.GITHUB_TOKEN) {
    throw new Error("Update feed publication requires this repository's tagged GitHub workflow.");
  }
  const request = async (path, {method = "GET", body, allowMissing = false} = {}) => {
    if (!path.startsWith(apiRoot + "/")) throw new Error("Unexpected repository.");
    const response = await fetch("https://api.github.com" + path, {
      method, headers: {Authorization: `Bearer ${process.env.GITHUB_TOKEN}`,
        Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28",
        "Content-Type": "application/json", "User-Agent": "Discord-to-VR-release"},
      ...(body ? {body: JSON.stringify(body)} : {}), signal: AbortSignal.timeout(30000),
    });
    if (allowMissing && response.status === 404) return null;
    if (!response.ok) throw new Error(`GitHub update feed request failed (${response.status}).`);
    return response.json();
  };
  const config = JSON.parse(await readFile("src-tauri/tauri.conf.json", "utf8"));
  const cargo = await readFile("src-tauri/Cargo.toml", "utf8");
  const pkg = JSON.parse(await readFile("package.json", "utf8"));
  const version = validateVersion(config, cargo, pkg, process.env.GITHUB_REF_NAME);
  const planPath = join(process.env.RUNNER_TEMP ?? "target", "update-feed-plan.json");
  if (process.argv.includes("--plan")) {
    const plan = distributionPlan(version, await readFeed(request));
    await mkdir(dirname(planPath), {recursive: true});
    await writeFile(planPath, JSON.stringify(plan));
    await appendFile(process.env.GITHUB_OUTPUT,
      `release_files<<DISCORD_VR_ASSETS\n${plan.files.join("\n")}\nDISCORD_VR_ASSETS\n`);
    console.log(`Release ${version}: ${plan.includeLegacyManifest ? "migration release with latest.json" : "installer only"}.`);
  } else if (process.argv.includes("--publish")) {
    const plan = JSON.parse(await readFile(planPath, "utf8"));
    if (plan.version !== version) throw new Error("Release plan version mismatch.");
    const update = JSON.parse(await readFile("latest.json", "utf8"));
    await publishFeed(request, update, plan);
    console.log(`Update feed published for ${version}.`);
  } else {
    throw new Error("Use --plan or --publish.");
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
