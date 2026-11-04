#!/usr/bin/env python3
"""Exercise comparison isolation/failure reporting with synthetic toolchains.
No real build, release artifact, network or reproducibility claim is made.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
SCRIPT=Path(__file__).resolve().parent/'release/compare-builds.sh'
with tempfile.TemporaryDirectory(prefix='nightfall-build-runner-test-') as tmp:
    root=Path(tmp); (root/'scripts/release').mkdir(parents=True); (root/'bin').mkdir()
    shutil.copyfile(SCRIPT,root/'scripts/release/compare-builds.sh')
    def tool(name,body):
        path=root/'bin'/name;path.write_text('#!/usr/bin/env python3\n'+body);path.chmod(0o700)
    tool('git', '''import sys,tarfile,io
if sys.argv[1]=='rev-parse': print('a'*40)
elif sys.argv[1]=='log': print('1700000000')
elif sys.argv[1]=='archive':
 with tarfile.open(fileobj=sys.stdout.buffer,mode='w|') as t:
  data=b'# synthetic lockfile\\n'; entry=tarfile.TarInfo('Cargo.lock');entry.size=len(data);t.addfile(entry,io.BytesIO(data))
else: sys.exit(2)
''')
    tool('rustc', "print('rustc synthetic-test-only')\n")
    tool('cargo', '''import os,pathlib,sys
assert all(x not in os.environ for x in ['CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','CARGO_BUILD_TARGET','CARGO_BUILD_RUSTFLAGS','RUSTC'])
target=pathlib.Path(os.environ['CARGO_TARGET_DIR'])
assert target.name=='target' and target.parent.name in ['a','b']
assert str(target) in os.environ['RUSTFLAGS']
assert '--offline' in sys.argv and '--locked' in sys.argv
if os.environ.get('TEST_BUILD_FAIL')=='yes': sys.exit(42)
(target/'release').mkdir(parents=True)
for name in ['nightfalld','nightfall-wallet','nightfall-nodekit']:
 (target/'release'/name).write_bytes(b'SYNTHETIC TEST ONLY '+name.encode()+(target.parent.name.encode() if os.environ.get('TEST_DIFFER')=='yes' else b''))
''')
    env=dict(os.environ,PATH=str(root/'bin')+os.pathsep+os.environ['PATH'],TMPDIR=str(root),CARGO_TARGET_DIR=str(root/'shared'),CARGO_ENCODED_RUSTFLAGS='hostile',RUSTC_WRAPPER='hostile',RUSTC_WORKSPACE_WRAPPER='hostile',RUSTC='hostile',CARGO_BUILD_TARGET='hostile',CARGO_BUILD_RUSTFLAGS='hostile')
    command=['bash',str(root/'scripts/release/compare-builds.sh')]
    success=subprocess.run(command,env=env,capture_output=True,text=True)
    assert success.returncode==0,success.stderr+success.stdout
    assert not (root/'shared').exists()
    reports=list((root/'output/release-compare').glob('*/report.md'));assert len(reports)==1
    out=reports[0].parent
    assert (out/'SHA256SUMS-a.txt').read_bytes()==(out/'SHA256SUMS-b.txt').read_bytes()
    # Remove only this disposable synthetic report before testing failure.
    shutil.rmtree(root/'output')
    failed=subprocess.run(command,env=dict(env,TEST_BUILD_FAIL='yes'),capture_output=True,text=True)
    assert failed.returncode!=0
    assert list((root/'output/release-compare').glob('*/FAILED.txt'))
    assert not list((root/'output/release-compare').glob('*/report.md'))
    shutil.rmtree(root/'output')
    mismatch=subprocess.run(command,env=dict(env,TEST_DIFFER='yes'),capture_output=True,text=True)
    assert mismatch.returncode!=0
    failure=list((root/'output/release-compare').glob('*/FAILED.txt'))[0]
    evidence=Path(failure.read_text().split('Temporary evidence: ')[1].strip())
    assert (evidence/'a/target/release/nightfalld').exists()
    assert (evidence/'b/target/release/nightfalld').exists()
print('Build comparison runner: independent targets, inherited environment isolation, both manifests and failed-build evidence passed (synthetic tools only).')
