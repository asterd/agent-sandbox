import { Sandbox } from "../../sdks/typescript/dist/index.js";

const daemonUrl = process.env.AGENTSANDBOX_DAEMON_URL ?? "http://127.0.0.1:7847";

await using sandbox = await Sandbox.create({
  runtime: "python",
  ttl: 60,
  daemonUrl,
});

const result = await sandbox.exec("python -c 'print(40 + 2)'");
const info = await sandbox.inspect();

if (result.exit_code !== 0) {
  throw new Error(result.stderr || result.stdout);
}
if (result.stdout.trim() !== "42") {
  throw new Error(`unexpected stdout: ${result.stdout}`);
}
if (info.status !== "running") {
  throw new Error(`unexpected status: ${info.status}`);
}
