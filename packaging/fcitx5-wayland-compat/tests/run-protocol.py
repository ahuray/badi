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

def run(module, baseline, disabled=False):
    root=Path(tempfile.mkdtemp(prefix='protocol-',dir=HERE))
    runtime=tempfile.TemporaryDirectory(prefix='badi-ack-')
    for name in ['home','config','data','cache','state','runtime','addons','share/addon']:
        (root/name).mkdir(parents=True,exist_ok=True)
    (root/'runtime').chmod(0o700)
    shutil.copy2(module,root/'addons/libwaylandim.so')
    shutil.copy2(HERE/'build/libackprobe.so',root/'addons/libackprobe.so')
    shutil.copy2('/usr/share/fcitx5/addon/waylandim.conf',root/'share/addon/waylandim.conf')
    (root/'share/addon/ackprobe.conf').write_text('[Addon]\nName=Private protocol probe\nType=SharedLibrary\nLibrary=libackprobe\nCategory=Module\nVersion=1\n[Addon/Dependencies]\n0=core:5.1.21\n')
    (root/'config/fcitx5/conf').mkdir(parents=True)
    (root/'config/fcitx5/conf/waylandim.conf').write_text('DetectApplication=False\nIdleDoneAcknowledgement='+('False' if disabled else 'True')+'\n')
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
        if baseline or disabled:
            assert len(commits())==0, commits()
            checks.append(('disabled compatibility option' if disabled else 'unpatched actual frontend')+' produces no activation acknowledgement')
        else:
            assert len(commits())==1 and commits()[-1]=={'event':'commit','serial':1,'latest':1,'preedit':0,'insert':0,'delete':0},commits()
            checks.append('activation receives exactly one bare latest-serial commit with zero mutation requests')
            quiet(.5);assert len(commits())==1
            checks.append('idle connection remains quiet rather than producing autonomous commits')
            command('surround');wait(lambda:len(commits())==2,'later surrounding-state ack')
            assert commits()[-1]['serial']==2 and not any(commits()[-1][key] for key in ['insert','delete','preedit'])
            checks.append('later surrounding-text publication receives its own bounded bare acknowledgement')
            before=len(commits());command('chain');wait(lambda:len(commits())>=before+8,'bounded publication feedback chain')
            quiet(.5);assert len(commits())==before+8
            assert not any(r[key] for r in commits()[before:] for key in ['insert','delete','preedit'])
            checks.append('eight queued serial-gated client publications drain and stop without a feedback loop or mutation')
            driver('P');wait(lambda:any(r['event']=='preedit' and r['bytes']==7 for r in records),'real preedit protocol request')
            before=len(commits());command('done');quiet();assert len(commits())==before
            checks.append('active sent preedit blocks bare acknowledgement on a newer done')
            driver('R');wait(lambda:len(commits())==before+1,'explicit preedit clear')
            before=len(commits());command('done');wait(lambda:len(commits())==before+1,'ack resumes after preedit clear')
            checks.append('acknowledgement resumes after explicit native preedit clear')
            driver('L');before=len(commits());command('done');quiet();assert len(commits())==before
            driver('S');command('done');wait(lambda:len(commits())==before+1,'ack after local preedit clear')
            checks.append('local engine preedit also blocks idle acknowledgement')
            driver('X');before=len(commits());command('done');quiet();assert len(commits())==before
            driver('Z');command('done');wait(lambda:len(commits())==before+1,'ack after compose reset')
            checks.append('real XCompose state blocks acknowledgement until reset')
            before=len(commits());driver('C');assert len(commits())==before+1 and commits()[-1]['insert']==1
            before=len(commits());driver('D');assert len(commits())==before+1 and commits()[-1]['delete']==1
            checks.append('ordinary explicit insertion and deletion remain single committed protocol operations')
            before=len(commits());command('deactivate');quiet();assert len(commits())==before
            checks.append('deactivation does not emit an idle commit')
            command('activate');wait(lambda:len(commits())==before+1,'fresh activation ack')
            assert commits()[-1]['serial']==commits()[-1]['latest']
            checks.append('new activation acknowledges fresh serial after focus loss')
            driver('P');command('deactivate');quiet();before=len(commits());command('activate')
            wait(lambda:len(commits())==before+1,'fresh activation after uncleared prior preedit')
            assert not any(commits()[-1][key] for key in ['insert','delete','preedit'])
            checks.append('activation resets old protocol preedit state before focus callbacks')
            driver('A');command('deactivate');quiet();before=len(commits());command('activate')
            wait(lambda:len(commits())>=before+1,'focus callback explicit commit');quiet()
            assert len(commits())==before+1 and commits()[-1]['insert']==1
            checks.append('a real focus callback commit suppresses a duplicate bare commit for the same serial')
            assert all(r['serial']==r['latest'] for r in commits())
        result={'passed':True,'baseline':baseline,'disabled':disabled,'checks':checks,'events':records,'process_ids':[p.pid for p in children]+[fcitx_pid], 'loaded_frontend':str(root/'addons/libwaylandim.so'),'boundary':'actual Fcitx5.1.21/frontend + synthetic private Wayland compositor + explicit fixture driver; no physical editor', 'normal_desktop_mutated':False}
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
    parser=argparse.ArgumentParser();parser.add_argument('--work-dir',type=Path,required=True);parser.add_argument('--baseline',action='store_true');parser.add_argument('--disabled',action='store_true');args=parser.parse_args()
    HERE=args.work_dir.resolve()
    run(HERE/'libwaylandim-baseline.so' if args.baseline else HERE/'build/libwaylandim.so',args.baseline,args.disabled)
