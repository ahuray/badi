#!/usr/bin/env python3
"""Synthetic compositor, actual installed Fcitx and private frontend overrides."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import threading
import time

HERE=None
VERSION=json.loads((Path(__file__).resolve().parents[1]/'manifest.json').read_text())['upstream_version']

def run(module, baseline):
    root=Path(tempfile.mkdtemp(prefix='protocol-',dir=HERE))
    runtime=tempfile.TemporaryDirectory(prefix='badi-ack-')
    for name in ['home','config','data','cache','state','runtime','addons','share/addon']:
        (root/name).mkdir(parents=True,exist_ok=True)
    (root/'runtime').chmod(0o700)
    shutil.copy2(module,root/'addons/libwaylandim.so')
    shutil.copy2(HERE/'build/libackprobe.so',root/'addons/libackprobe.so')
    shutil.copy2('/usr/share/fcitx5/addon/waylandim.conf',root/'share/addon/waylandim.conf')
    (root/'share/addon/ackprobe.conf').write_text('[Addon]\nName=Private protocol probe\nType=SharedLibrary\nLibrary=libackprobe\nCategory=Module\nVersion=1\n[Addon/Dependencies]\n0=core:'+VERSION+'\n')
    (root/'config/fcitx5/conf').mkdir(parents=True)
    (root/'config/fcitx5/conf/waylandim.conf').write_text('DetectApplication=False\n')
    (root/'config/fcitx5/profile').write_text('[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=keyboard-us\n[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n[GroupOrder]\n0=Default\n')
    env=dict(os.environ)
    for key in ['DISPLAY','DBUS_SESSION_BUS_ADDRESS','AT_SPI_BUS_ADDRESS','HYPRLAND_INSTANCE_SIGNATURE','XDG_CURRENT_DESKTOP','GTK_IM_MODULE','QT_IM_MODULE']:
        env.pop(key,None)
    env.update(HOME=str(root/'home'),XDG_CONFIG_HOME=str(root/'config'),XDG_DATA_HOME=str(root/'data'),
               XDG_CACHE_HOME=str(root/'cache'),XDG_STATE_HOME=str(root/'state'),XDG_RUNTIME_DIR=runtime.name,
               WAYLAND_DISPLAY='badi-probe',XDG_SESSION_TYPE='wayland',
               FCITX_ADDON_DIRS=f'{root}/addons:/usr/lib/fcitx5',FCITX_DATA_DIRS=f'{root}/share:/usr/share/fcitx5')
    records=[]; fcitx_lines=[]; children=[]; readers=[]
    def consume(process,kind):
        with (root/f'{kind}.log').open('w') as log:
            for line in process.stdout:
                log.write(line);log.flush()
                if kind=='compositor': records.append(json.loads(line))
                else: fcitx_lines.append(line.rstrip())
    def launch(args,kind,**kwargs):
        proc=subprocess.Popen(args,env=env,stdout=subprocess.PIPE,stderr=(root/f'{kind}.stderr').open('w'),text=True,start_new_session=True,**kwargs)
        children.append(proc)
        thread=threading.Thread(target=consume,args=(proc,kind),daemon=True);thread.start();readers.append(thread)
        return proc
    def wait(predicate,label,seconds=5):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            assert all(p.poll() is None for p in children), f'private process exited: {label}; {root}'
            if predicate():return
            time.sleep(.01)
        raise AssertionError(f'timeout {label}; {root}')
    def quiet(seconds=.25):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            assert all(p.poll() is None for p in children), f'private process exited; {root}'
            time.sleep(.01)
    def commits():return [r for r in records if r['event']=='commit']
    checks=[]
    readfd,writefd=os.pipe()
    try:
        compositor=launch([str(HERE/'build/probe-compositor'),'badi-probe'],'compositor',stdin=subprocess.PIPE)
        wait(lambda:any(r['event']=='ready' for r in records),'private compositor ready')
        env['BADI_PRIVATE_PROBE_FD']=str(readfd)
        fcitx=launch(['dbus-run-session','--','/usr/bin/fcitx5','-D','--disable=all','--enable=wayland,waylandim,keyboard,ackprobe'],'fcitx',pass_fds=(readfd,),stdin=subprocess.DEVNULL)
        wait(lambda:any(r['event']=='input_method' for r in records),'actual frontend connects')
        wait(lambda:any(line.startswith('PROBE_PID ') for line in fcitx_lines),'fixture process identity')
        fcitx_pid=int(next(line.split()[1] for line in fcitx_lines if line.startswith('PROBE_PID ')))
        assert str(root/'addons/libwaylandim.so') in Path(f'/proc/{fcitx_pid}/maps').read_text()
        def command(value):compositor.stdin.write(value+'\n');compositor.stdin.flush()
        def driver(value):
            count=sum(line==f'PROBE_ACTION {value}' for line in fcitx_lines)
            os.write(writefd,value.encode())
            wait(lambda:sum(line==f'PROBE_ACTION {value}' for line in fcitx_lines)>count,'driver '+value)
            quiet(.08)
        command('activate');wait(lambda:'PROBE_FOCUS' in fcitx_lines,'focused actual input context')
        quiet()
        def latest_preedit():return next(r for r in reversed(records) if r['event']=='preedit')
        def bare(record):return not any(record[key] for key in ['insert','delete','preedit'])
        if baseline:
            assert len(commits())==0, commits()
            checks.append('unpatched actual frontend produces no activation acknowledgement')
        else:
            assert len(commits())==1 and commits()[-1]=={'event':'commit','serial':1,'latest':1,'preedit':0,'insert':0,'delete':0},commits()
            checks.append('activation receives exactly one bare latest-serial commit with zero mutation requests')
            quiet(.5);assert len(commits())==1
            checks.append('idle connection remains quiet rather than producing autonomous commits')
            command('surround');wait(lambda:len(commits())==2,'later surrounding-state refresh')
            assert commits()[-1]['serial']==2 and bare(commits()[-1])
            checks.append('later surrounding-text publication receives its own bare refresh commit')
            before=len(commits());command('chain');wait(lambda:len(commits())>=before+8,'bounded publication feedback chain')
            quiet(.5);assert len(commits())==before+8
            assert all(bare(r) for r in commits()[before:])
            checks.append('eight queued serial-gated client publications drain and stop without a feedback loop or mutation')
            before=len(commits())
            for _ in range(20):command('done')
            wait(lambda:len(commits())>=before+20,'one refresh per done');quiet(.5)
            storm=commits()[before:];assert len(storm)==20 and all(bare(r) for r in storm)
            assert [r['serial'] for r in storm]==list(range(storm[0]['serial'],storm[0]['serial']+20)) and storm[-1]['serial']==storm[-1]['latest']
            checks.append('twenty queued bare dones yield exactly twenty ordered bare refreshes and no self-sustaining output')
            driver('P');wait(lambda:any(r['event']=='preedit' and r['bytes']==7 for r in records),'real preedit protocol request')
            preedits=sum(r['event']=='preedit' for r in records)
            before=len(commits());command('done');wait(lambda:len(commits())==before+1,'refresh with active preedit');quiet()
            assert len(commits())==before+1 and commits()[-1]['preedit']==1 and not commits()[-1]['insert'] and not commits()[-1]['delete']
            assert sum(r['event']=='preedit' for r in records)==preedits+1 and latest_preedit()['bytes']==7
            checks.append('active client preedit is re-sent with the refresh commit instead of being cleared')
            driver('R');wait(lambda:len(commits())==before+2,'explicit preedit clear')
            before=len(commits());command('done');wait(lambda:len(commits())==before+1,'bare refresh after preedit clear');quiet()
            assert len(commits())==before+1 and bare(commits()[-1])
            checks.append('after an explicit native preedit clear the refresh is bare')
            driver('L');before=len(commits());command('done');wait(lambda:len(commits())==before+1,'refresh with local preedit');quiet()
            assert len(commits())==before+1 and bare(commits()[-1])
            driver('S')
            checks.append('local engine preedit is not sent to the client; its refresh stays bare')
            driver('X');before=len(commits());command('done');wait(lambda:len(commits())==before+1,'refresh while composing');quiet()
            assert len(commits())==before+1 and bare(commits()[-1])
            driver('Z')
            checks.append('XCompose state without client preedit receives one bare refresh')
            before=len(commits());driver('C');assert len(commits())==before+1 and commits()[-1]['insert']==1
            before=len(commits());driver('D');assert len(commits())==before+1 and commits()[-1]['delete']==1
            checks.append('ordinary explicit insertion and deletion remain single committed protocol operations')
            before=len(commits());command('deactivate');quiet();assert len(commits())==before
            checks.append('deactivation does not emit a refresh commit')
            command('activate');wait(lambda:len(commits())==before+1,'fresh activation refresh');quiet()
            assert len(commits())==before+1 and bare(commits()[-1]) and commits()[-1]['serial']==commits()[-1]['latest']
            checks.append('new activation refreshes the fresh serial after focus loss')
            driver('P');command('deactivate');quiet();before=len(commits());command('activate')
            wait(lambda:len(commits())==before+1,'activation after uncleared prior preedit');quiet()
            assert len(commits())==before+1 and bare(commits()[-1])
            checks.append('preedit left in a previous field is not re-sent to the newly activated field')
            driver('A');command('deactivate');quiet();before=len(commits());command('activate')
            wait(lambda:len(commits())>=before+2,'focus callback commit plus refresh');quiet()
            assert len(commits())==before+2 and commits()[-2]['insert']==1 and bare(commits()[-1])
            assert commits()[-2]['serial']==commits()[-1]['serial']
            checks.append('a focus callback commit is followed by exactly one same-serial bare refresh')
            assert all(r['serial']==r['latest'] for r in commits() if r not in storm[:-1])
        result={'passed':True,'baseline':baseline,'checks':checks,'events':records,'process_ids':[p.pid for p in children]+[fcitx_pid], 'loaded_frontend':str(root/'addons/libwaylandim.so'),'boundary':f'actual Fcitx {VERSION} frontend + synthetic private Wayland compositor + explicit fixture driver; no physical editor', 'normal_desktop_mutated':False}
        (root/'result.json').write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps({'report':str(root/'result.json'),'checks':checks}),flush=True)
    except BaseException:
        (root/'result.json').write_text(json.dumps({'passed':False,'checks':checks,'events':records},indent=2)+'\n')
        raise
    finally:
        os.close(readfd);os.close(writefd)
        for proc in reversed(children):
            if proc.poll() is None:
                os.killpg(proc.pid,signal.SIGTERM)
                try:proc.wait(timeout=3)
                except subprocess.TimeoutExpired:os.killpg(proc.pid,signal.SIGKILL);proc.wait(timeout=3)
        for thread in readers:thread.join(timeout=2)
        runtime.cleanup()

if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--work-dir',type=Path,required=True);parser.add_argument('--baseline',action='store_true');args=parser.parse_args()
    HERE=args.work_dir.resolve()
    run(HERE/'libwaylandim-baseline.so' if args.baseline else HERE/'build/libwaylandim.so',args.baseline)
