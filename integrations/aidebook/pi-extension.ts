// Generated installation supplies config.json beside this extension. No shell is used.
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

export default function (pi: any) {
  const config = JSON.parse(readFileSync(fileURLToPath(new URL("./config.json", import.meta.url)), "utf8"));
  for (const tool of config.tools) {
    pi.registerTool({
      name: `aidebook_${tool.name.replaceAll(".", "_")}`,
      label: `Aidebook ${tool.name}`,
      description: tool.description,
      parameters: tool.inputSchema,
      async execute(_id: string, params: unknown, signal: AbortSignal) {
        const text = await new Promise<string>((resolve, reject) => {
          const child = spawn(config.command, ["--aidebook-agent", "call", tool.name, "--data-dir", config.dataDir], { shell: false, stdio: ["pipe", "pipe", "pipe"] });
          let output = "";
          let bytes = 0;
          let settled = false;
          const finish = (error?: Error) => {
            if (settled) return;
            settled = true;
            clearTimeout(timer);
            signal?.removeEventListener("abort", abort);
            if (error) { child.kill(); reject(error); } else resolve(output);
          };
          const abort = () => finish(new Error("Aidebook request cancelled"));
          const timer = setTimeout(() => finish(new Error("Aidebook timed out; check that the desktop app is running")), 30000);
          signal?.addEventListener("abort", abort, { once: true });
          if (signal?.aborted) abort();
          child.on("error", () => finish(new Error("Aidebook bridge could not start; reinstall from agent settings")));
          child.stdout.setEncoding("utf8");
          child.stdout.on("data", (chunk: string) => {
            bytes += Buffer.byteLength(chunk);
            if (bytes > 8 * 1024 * 1024) return finish(new Error("Aidebook response exceeds the size limit"));
            output += chunk;
          });
          child.stderr.resume();
          child.on("close", (code: number) => finish(code === 0 ? undefined : new Error("Aidebook rejected the request; check app status and tool arguments")));
          child.stdin.on("error", () => finish(new Error("Aidebook input could not be sent")));
          child.stdin.end(JSON.stringify(params));
        });
        return { content: [{ type: "text", text }], details: { provider: "aidebook" } };
      },
    });
  }
}
