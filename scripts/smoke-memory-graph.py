#!/usr/bin/env python3
"""Exercise real local Core/CLI/MCP binaries against temporary fixture data only."""
import json, os, pathlib, subprocess, tempfile, time
repo=pathlib.Path(__file__).resolve().parents[1]
bin_dir=pathlib.Path(os.environ.get('AIDEBOOK_BIN_DIR', str(repo/'src-tauri/target/debug')))
fixture=repo/'src-tauri/fixtures/obsidian.json'
with tempfile.TemporaryDirectory(prefix='aidebook-memory-cli-') as tmp:
    d=pathlib.Path(tmp); sock=d/'core.sock'; token=d/'core.token'
    owner=subprocess.Popen([str(bin_dir/'aidebook-core'),'--db',str(d/'core.sqlite'),'--socket',str(sock),'--token-file',str(token)],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    try:
        for _ in range(100):
            if sock.exists() and token.exists(): break
            if owner.poll() is not None: raise RuntimeError('owner exited before readiness')
            time.sleep(.03)
        env={**os.environ,'AIDEBOOK_IPC_SOCKET':str(sock),'AIDEBOOK_IPC_TOKEN_FILE':str(token)}
        def cli(command,params):
            p=subprocess.run([str(bin_dir/'aidebook-cli'),*command,'--params',json.dumps(params)],env=env,text=True,capture_output=True,timeout=15)
            if p.returncode: raise AssertionError(f'{command}: {p.stderr}')
            assert not p.stderr.strip(),(command,p.stderr)
            return json.loads(p.stdout)
        cli(['sources','refresh'],{'fixture_path':str(fixture)})
        source=json.loads(fixture.read_text())['snapshots'][0]['source']
        capture=cli(['observation','capture'],{'session_id':'parent-smoke','body':'Keep local review before promotion','evidence':[source],'actor':'agent','idempotency_key':'smoke:capture'})
        distilled=cli(['candidate','distill'],{'observation_id':capture['observation']['id'],'body':'Keep local review before promotion','reason':'Explicit fixture decision','author':'agent','claim_type':'decision','expected_version':1,'idempotency_key':'smoke:distill'})
        proposed=cli(['candidate','propose'],{'id':distilled['candidate']['id'],'expected_version':distilled['candidate']['version'],'idempotency_key':'smoke:propose'})
        assert proposed['candidate']['state']=='proposed'
        listed=cli(['candidate','list'],{'state':'proposed'})
        assert len(listed)==1
        packet=cli(['context','query'],{'query':'release'})
        assert packet['api_version']=='context.query.v1' and packet['sources']
        requests=[{'jsonrpc':'2.0','id':1,'method':'initialize','params':{}},{'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}},{'jsonrpc':'2.0','id':3,'method':'tools/call','params':{'name':'context.query.v1','arguments':{'query':'release'}}},{'jsonrpc':'2.0','id':4,'method':'tools/call','params':{'name':'candidate.accept','arguments':{'id':proposed['candidate']['id']}}}]
        mcp=subprocess.run([str(bin_dir/'aidebook-cli'),'mcp','serve','--stdio'],input='\n'.join(map(json.dumps,requests))+'\n',env=env,text=True,capture_output=True,timeout=15)
        assert mcp.returncode==0 and not mcp.stderr.strip(),mcp.stderr
        responses=[json.loads(line) for line in mcp.stdout.splitlines()]
        tools={t['name'] for t in responses[1]['result']['tools']}
        assert {'context.search','context.get','memory.upsert','memory.retract','sources.refresh','connections.status','context.query.v1','observation.capture','candidate.distill','candidate.propose','candidate.list'} <= tools
        assert 'candidate.accept' not in tools and 'candidate.reject' not in tools
        assert responses[2]['result']['isError']==False
        denied=responses[3]; assert 'error' in denied or denied.get('result',{}).get('isError')==True
        print(json.dumps({'cli_capture_distill_propose_list':'passed','context_query':'passed','mcp_stdio_tools':len(tools),'mcp_query':'passed','mcp_human_review_boundary':'passed','database':'temporary fixture only'}))
    finally:
        owner.terminate()
        try: owner.wait(timeout=5)
        except subprocess.TimeoutExpired: owner.kill();owner.wait()
