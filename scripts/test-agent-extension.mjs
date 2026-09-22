// Exercises the shipped Pi extension with real child processes in a temporary directory.
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import ts from 'typescript';
const root = await mkdtemp(join(tmpdir(), 'aidebook-pi-'));
try {
  const bridge = join(root, 'bridge with spaces');
  await writeFile(bridge, `#!/usr/bin/env node\nlet input='';process.stdin.on('data',b=>input+=b);process.stdin.on('end',()=>{const value=JSON.parse(input);if(value.wait)return setTimeout(()=>{},60000);process.stdout.write(JSON.stringify({params:value,args:process.argv.slice(2)}));});\n`, { mode:0o700 });
  const source = readFileSync('integrations/aidebook/pi-extension.ts','utf8');
  const output = ts.transpileModule(source,{ compilerOptions:{ target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022 } }).outputText;
  await writeFile(join(root,'extension.mjs'),output);
  const tools=JSON.parse(execFileSync(resolve('src-tauri/target/debug/aidebook'),['--aidebook-agent','describe'],{encoding:'utf8'})).tools;
  await writeFile(join(root,'config.json'),JSON.stringify({command:bridge,dataDir:'/test data',tools}));
  const definitions=[];
  const extension=(await import(pathToFileURL(join(root,'extension.mjs')))).default;
  extension({registerTool:tool=>definitions.push(tool)});
  assert.equal(definitions.length,tools.length);
  assert(definitions.some(t=>t.name==='aidebook_plugins_update'));
  assert(!definitions.some(t=>t.name==='aidebook_candidate_accept'));
  const search=definitions.find(t=>t.name==='aidebook_context_search');
  const params={query:'한글 본문 $(never-execute) `literal` "quoted"'};
  const result=await search.execute('id',params,new AbortController().signal);
  const received=JSON.parse(result.content[0].text);
  assert.deepEqual(received.params,params);
  assert.deepEqual(received.args,['--aidebook-agent','call','context.search','--data-dir','/test data']);
  const controller=new AbortController();
  const pending=search.execute('cancel',{wait:true},controller.signal);
  controller.abort();
  await assert.rejects(pending,/cancelled/);
  console.log(`Pi extension: ${definitions.length} tools, argument isolation, Unicode and cancellation passed (temporary process harness).`);
} finally {await rm(root,{recursive:true,force:true});}
