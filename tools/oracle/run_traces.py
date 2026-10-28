import json, os, subprocess, sys
# Runs the user's own copy of the game headless and records what its objects do each tick.
# Usage: python3 run_traces.py WORKDIR [trace name ...]
# WORKDIR gets a copy of the game files (engine + pack + steam libs), override.cfg and the
# traces. Nothing is written to the game folder. Copy WORKDIR/traces/*.json to data/oracle/.
HERE=os.path.dirname(os.path.abspath(__file__))
O=os.path.abspath(sys.argv[1]); sys.argv=sys.argv[:1]+sys.argv[2:]
specs={
 'ninja_fireball':dict(char='Ninja',state='Fireball',data={'x':100,'y':0}),
 'ninja_air_fireball':dict(char='Ninja',state='AirFireball',data={'x':71,'y':71},air=True),
 'ninja_grappling_hook':dict(char='Ninja',state='GrapplingHook',data={'x':100,'y':0}),
 'robo_flamethrower':dict(char='Robot',state='Flamethrower',char_distance=80,ticks=160),
 'robo_flamethrower2':dict(char='Robot',state='Flamethrower2'),
 'robo_loic':dict(char='Robot',state='LOIC',data={'Self':False,'Dir':{'x':0,'y':0},'Delay':{'x':30}}),
 'robo_grenade':dict(char='Robot',state='Grenade',data={'x':0,'y':-100}),
 'robo_missile':dict(char='Robot',state='Missile',data={'x':1,'y':0}),
 'wizard_spark_bomb':dict(char='Wizard',state='SparkBomb',ticks=240),
 'wizard_vile_clutch':dict(char='Wizard',state='VileClutch',data={'x':80}),
 'mutant_caustic_spike':dict(char='Mutant',state='CausticSpike',data={'x':100,'y':0}),
 'mutant_gas_bomb':dict(char='Mutant',state='GasBomb',data={'x':100,'y':0},ticks=200),
 'mutant_bite_throw':dict(char='Mutant',state='BiteThrow'),
 'mutant_shockwave':dict(char='Mutant',state='Shockwave'),
 'cowboy_foresight_neutral':dict(char='Cowboy',state='ForesightNeutral',data={'x':100,'y':0}),
}
only=sys.argv[1:]
G=os.environ.get('YEMI_GAME',os.path.expanduser('~/.local/share/Steam/steamapps/common/YourOnlyMoveIsHUSTLE'))
import shutil
os.makedirs(f'{O}/run',exist_ok=True)
for f in ['YourOnlyMoveIsHUSTLE.x86_64','YourOnlyMoveIsHUSTLE.pck','libsteam_api.so','tbfg.so']:
    if not os.path.exists(f'{O}/run/{f}'): shutil.copy(f'{G}/{f}',f'{O}/run/{f}')
for f in ['trace.gd','trace.tscn']: shutil.copy(f'{HERE}/{f}',f'{O}/run/{f}')
open(f'{O}/run/override.cfg','w').write('[application]\nrun/main_scene="res://trace.tscn"\n')
for name,s in specs.items():
    if only and name not in only: continue
    s=dict(dict(ticks=100),**s,out=f'{O}/traces/{name}.json')
    os.makedirs(f'{O}/traces',exist_ok=True)
    sp=f'{O}/run/spec_{name}.json'; json.dump(s,open(sp,'w'))
    r=subprocess.run(['./YourOnlyMoveIsHUSTLE.x86_64','--audio-driver','Dummy','--no-window'],cwd=f'{O}/run',env=dict(os.environ,YEMI_TRACE_SPEC=sp),capture_output=True,text=True,timeout=120)
    ok='TRACE done' in r.stdout
    errs=[l for l in (r.stdout+r.stderr).split('\n') if 'SCRIPT ERROR' in l]
    print(name,'ok' if ok else 'FAILED',errs[:2],flush=True)
