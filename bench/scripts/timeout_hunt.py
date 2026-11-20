import json,subprocess,os,re,time,sys
es=sys.argv[1]; out=open('timeout_hunt.jsonl','a')
C=[c for c in json.load(open(os.environ.get('CORPUS', 'corpus.json'))) if c['recorded_s']>=1700]
for c in C:
    flags=([] if '-D' in c['options'] else ['-deadlock'])
    t=time.time()
    p=subprocess.run([es,'-config',c['cfg'],os.path.basename(c['spec']),'-hunt','budget=300,max=20000000','-seed','1']+flags,cwd=os.path.dirname(c['spec']),capture_output=True,text=True,env=dict(os.environ,EVERYSTATE_SIM_REPLAY='1'))
    txt=p.stdout; tl=re.findall(r'^State (\d+):',txt,re.M); v=re.search(r'(Invariant \S+ is violated|Deadlock reached|Action property \S+ is violated)',txt); h=re.search(r'Hunt finished \(([^)]*)\): (\d+) distinct',txt)
    rec=dict(cfg=c['cfg'],run=c['run'],code=p.returncode,secs=round(time.time()-t,1),violation=v.group(0) if v else None,trace_len=max(map(int,tl)) if tl else None,stop=h.group(1) if h else None,archived=int(h.group(2)) if h else None,replay_failed='not a behavior' in txt,tlc_recorded_min=round(c['recorded_s']/60,1))
    out.write(json.dumps(rec)+'\n'); out.flush(); print(rec,flush=True)
