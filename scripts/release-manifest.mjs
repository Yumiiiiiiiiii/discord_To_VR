import {readFile,writeFile,readdir} from "node:fs/promises";
import {resolve,join} from "node:path";
import {fileURLToPath} from "node:url";

export const repository="Yumiiiiiiiiii/discord_To_VR";
export function validateVersion(config, cargo, pkg, tag) {
  const version=config.version;
  if(!/^\d+\.\d+\.\d+$/.test(version))throw new Error("Stable releases require a numeric major.minor.patch version.");
  const cargoVersion=cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if(cargoVersion!==version||pkg.version!==version)throw new Error("Tauri, desktop Cargo and package.json versions must match.");
  if(tag&&tag!==`v${version}`)throw new Error("The release tag must match the application version.");
  if(config.bundle.createUpdaterArtifacts!==true||!config.plugins.updater.requireSignedVersion)throw new Error("Signed, version-bound updater artifacts must be enabled.");
  const endpoints=config.plugins.updater.endpoints;
  if(endpoints.length!==1||endpoints[0]!==`https://github.com/${repository}/releases/latest/download/latest.json`)throw new Error("Unexpected updater endpoint.");
  return version;
}
export function manifest(version,tag,filename,signature,date=new Date()) {
  if(tag!==`v${version}`||filename!==`Discord to VR_${version}_x64-setup.exe`)throw new Error("Installer, version and tag must describe the same release.");
  const decoded=Buffer.from(signature.trim(),"base64").toString("utf8");
  if(!decoded.includes(`version:${version}`))throw new Error("The updater signature must be bound to this release version.");
  // GitHub normalizes spaces to dots in uploaded release asset names.
  const publishedFilename=filename.replaceAll(' ','.');
  return {
    version, notes:`Discord to VR ${version}。変更内容は GitHub Releases をご覧ください。`,pub_date:date.toISOString(),
    platforms:{"windows-x86_64":{url:`https://github.com/${repository}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(publishedFilename)}`,signature:signature.trim()}},
  };
}
async function main() {
  const config=JSON.parse(await readFile("src-tauri/tauri.conf.json","utf8"));
  const cargo=await readFile("src-tauri/Cargo.toml","utf8");
  const pkg=JSON.parse(await readFile("package.json","utf8"));
  const tag=process.env.GITHUB_REF_TYPE==="tag"?process.env.GITHUB_REF_NAME:undefined;
  const version=validateVersion(config,cargo,pkg,tag);
  if(process.env.GITHUB_REPOSITORY&&process.env.GITHUB_REPOSITORY!==repository)throw new Error("Forks must configure their own updater endpoint and signing key before releasing.");
  if(process.argv.includes("--validate")) { console.log(`Release version validated: ${version}`);return; }
  const releaseTag=tag??`v${version}`;
  const directory=resolve(process.env.CARGO_TARGET_DIR??"target/tauri","release/bundle/nsis");
  const expected=`Discord to VR_${version}_x64-setup.exe`;
  const files=await readdir(directory);
  if(!files.includes(expected)||!files.includes(`${expected}.sig`))throw new Error("Missing signed installer.");
  const signature=await readFile(join(directory,`${expected}.sig`),"utf8");
  await writeFile("latest.json",JSON.stringify(manifest(version,releaseTag,expected,signature),null,2)+"\n");
  console.log(`Updater manifest generated for ${version}`);
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url))await main();
