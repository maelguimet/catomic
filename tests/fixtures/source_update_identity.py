"""Exercise the real source updater with only local Git and temporary install stubs.

The wrapper replaces remote manifest/relation lookups and executable replacement;
the production source updater, workspaces, process runner, and build identity are
compiled unchanged. Cargo is stubbed to assert the exact requested clean commit.
"""

import atexit
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


repo, deps, package_version = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
fixture = tempfile.TemporaryDirectory(prefix="catomic-source-identity-")
atexit.register(fixture.cleanup)
root = Path(fixture.name)
real_git = shutil.which('git')
assert real_git is not None
checkout = root / 'checkout'
checkout.mkdir()
env = {**os.environ, 'GIT_CONFIG_GLOBAL': '/dev/null', 'GIT_CONFIG_NOSYSTEM': '1'}
def git(*args):
    return subprocess.check_output(
        [real_git, '-c', 'core.hooksPath=/dev/null', *args],
        cwd=checkout, env=env, text=True, timeout=10,
    ).strip()
git('init', '-q', '-b', 'master')
git('config', 'user.name', 'Fixture')
git('config', 'user.email', 'fixture@example.invalid')
(checkout / 'source').write_text('old\n')
git('add', '.')
git('commit', '-qm', 'old')
old = git('rev-parse', 'HEAD')
(checkout / 'source').write_text('current\n')
git('commit', '-qam', 'current')
current = git('rev-parse', 'HEAD')
git('remote', 'add', 'origin', 'https://github.com/maelguimet/catomic.git')
bin_dir = root / 'bin'
bin_dir.mkdir()
def executable(name, content):
    path = bin_dir / name
    path.write_text(content)
    path.chmod(0o700)
executable('git', '#!' + sys.executable + '''
import os, subprocess, sys
args=sys.argv[1:]
official='https://github.com/maelguimet/catomic.git'
if 'ls-remote' in args:
    assert official in args
    print(os.environ['FIXTURE_REMOTE_SHA']+'\\trefs/heads/master')
    sys.exit(0)
args=[os.environ['FIXTURE_CHECKOUT'] if arg==official else arg for arg in args]
assert not any('://' in arg for arg in args)
assert 'clone' not in args
assert all(index > 0 and args[index - 1] == 'stash'
           for index, arg in enumerate(args) if arg == 'push')
if 'fetch' in args:
    assert os.environ['FIXTURE_CHECKOUT'] in args
sys.exit(subprocess.call([os.environ['FIXTURE_GIT'],*args]))
''')
executable('cargo', '#!' + sys.executable + '''
import os, pathlib, subprocess, sys
if sys.argv[1:]==['--version']:
    print('cargo fixture')
    sys.exit(0)
assert sys.argv[1:]==['build','--release','--locked']
sha=subprocess.check_output([os.environ['FIXTURE_GIT'],'rev-parse','HEAD'],text=True).strip()
assert sha==os.environ['FIXTURE_REMOTE_SHA']==os.environ['CATOMIC_BUILD_COMMIT']
assert os.environ['CATOMIC_BUILD_DIRTY']=='0'
assert subprocess.check_output([os.environ['FIXTURE_GIT'],'status','--porcelain'],text=True)==''
target=pathlib.Path(os.environ['CARGO_TARGET_DIR'])/'release/catomic'
target.parent.mkdir(parents=True)
target.write_text("#!/bin/sh\\nif [ \\"$1\\" = --version ]; then printf '%s\\\\n' 'catomic "+os.environ['FIXTURE_PACKAGE_VERSION']+" (commit "+sha[:12]+")'; fi\\n".replace('\\"','"'))
target.chmod(0o700)
pathlib.Path(os.environ['FIXTURE_BUILD_LOG']).write_text(sha)
''')

harness = Path(__file__).with_suffix('.rs').read_text().replace('REPO', str(repo))
(root / 'source').mkdir()
shutil.copyfile(repo / 'src/update/source.rs', root / 'source/mod.rs')
shutil.copyfile(repo / 'src/update/source/workspace.rs', root / 'source/workspace.rs')
harness = harness.replace(str(repo / 'src/update/source.rs'),str(root / 'source/mod.rs'))
(root / 'harness.rs').write_text(harness)
# Cargo has already built libc for this test target. Prefer the latest artifact
# when a reused target directory contains older compiler/profile fingerprints.
libc = max(deps.glob('liblibc-*.rlib'), key=lambda path: path.stat().st_mtime_ns)
cases = [
    ('old', old, '0', True),
    ('current', current, '0', False),
    ('dirty', current, '1', True),
    ('unknown-state', current, 'unknown', None),
    ('missing', 'unknown', 'unknown', None),
]
for standalone in [False, True]:
    for name, commit, state, available in cases:
        label=('standalone-' if standalone else 'retained-')+name
        binary=root/label
        compile_env = {
            **env,
            'CARGO_MANIFEST_DIR': str(checkout),
            'CARGO_PKG_VERSION': package_version,
            'CATOMIC_BUILD_COMMIT': commit,
            'CATOMIC_BUILD_DIRTY': state,
            'CATOMIC_SOURCE_DIR': '' if standalone else str(checkout),
        }
        subprocess.run(
            ['rustc', '--edition=2021', str(root / 'harness.rs'), '-o', str(binary),
             '--extern', f'libc={libc}', '-L', f'dependency={deps}'],
            env=compile_env, check=True, timeout=60,
        )
        run_env = {
            **env,
            'PATH': str(bin_dir) + os.pathsep + env['PATH'],
            'FIXTURE_GIT': real_git,
            'FIXTURE_PACKAGE_VERSION': package_version,
            'FIXTURE_REMOTE_SHA': current,
            'FIXTURE_CHECKOUT': str(checkout),
            'FIXTURE_ROOT': str(root),
            'FIXTURE_INSTALL': str(root / 'installed'),
            'FIXTURE_BUILD_LOG': str(root / 'built'),
            'TMPDIR': str(root),
        }
        for check in [True,False]:
            for p in [root/'installed',root/'built']:
                p.unlink(missing_ok=True)
            (checkout/'notes').write_text('preserve untracked\n')
            (checkout/'source').write_text('preserve staged\n')
            git('add','source')
            before_status=git('status','--porcelain=v1')
            output = subprocess.run(
                [str(binary), *(['--check'] if check else [])],
                env=run_env, text=True, capture_output=True, timeout=30,
            )
            print(label, 'check' if check else 'apply', '\n'+output.stdout+output.stderr,flush=True)
            assert output.returncode==0
            identity=next(line.removeprefix('binary identity: catomic ') for line in output.stdout.splitlines() if line.startswith('binary identity: '))
            reported=next(line.removeprefix('current version: ').removeprefix('catomic ') for line in output.stdout.splitlines() if line.startswith('current version: '))
            assert reported==identity, 'installed version must describe the binary, not checkout HEAD'
            if check:
                expected='unknown' if available is None else ('yes' if available else 'no')
                assert 'update available: '+expected in output.stdout
                assert not (root/'installed').exists()
            elif available is not False:
                assert (root/'built').exists(), 'candidate build was skipped for an older or unverified binary'
                assert (root/'built').read_text()==current
                assert (root/'installed').exists()
                assert 'already current' not in output.stdout
            else:
                assert 'already current' in output.stdout
                assert not (root/'built').exists()
            assert git('status','--porcelain=v1')==before_status
            assert (checkout/'notes').read_text()=='preserve untracked\n'
            assert (checkout/'source').read_text()=='preserve staged\n'
            assert git('rev-parse','HEAD')==current
            assert git('stash','list')==''
print('All 20 source update identity fixture paths passed')
