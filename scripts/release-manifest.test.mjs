import {test} from "node:test";
import assert from "node:assert/strict";
import {manifest,validateVersion} from "./release-manifest.mjs";
const config={version:"0.2.0",bundle:{createUpdaterArtifacts:true},plugins:{updater:{requireSignedVersion:true,endpoints:["https://github.com/Yumiiiiiiiiii/discord_To_VR/releases/latest/download/latest.json"]}}};
test("release tags must match the app and installer versions",()=>{
  assert.equal(validateVersion(config,'version = "0.2.0"',{version:"0.2.0"},"v0.2.0"),"0.2.0");
  assert.throws(()=>validateVersion(config,'version = "0.2.0"',{version:"0.2.0"},"v0.3.0"));
  assert.throws(()=>validateVersion(config,'version = "0.1.0"',{version:"0.2.0"}));
});
test("manifest matches published GitHub filenames and rejects invalid signatures or installers",()=>{
  const signature=Buffer.from("trusted comment: version:0.2.0\n").toString("base64");
  const output=manifest("0.2.0","v0.2.0","Discord to VR_0.2.0_x64-setup.exe",signature,new Date("2026-10-08T00:00:00Z"));
  assert.equal(output.platforms["windows-x86_64"].signature,signature);
  assert.equal(output.platforms["windows-x86_64"].url,"https://github.com/Yumiiiiiiiiii/discord_To_VR/releases/download/v0.2.0/Discord.to.VR_0.2.0_x64-setup.exe");
  assert.throws(()=>manifest("0.2.0","v0.2.0","old-setup.exe",signature));
  assert.throws(()=>manifest("0.2.1","v0.2.1","Discord to VR_0.2.1_x64-setup.exe",signature));
});
