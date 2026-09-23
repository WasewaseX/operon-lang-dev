# -*- mode: python ; coding: utf-8 -*-
# PyInstaller spec — bundles the GenomeLab demo with the Python oracle runtime.
# This is the bootstrap packaging path; the native binary is bin/operon.
# Build:  pyinstaller packaging/genomelab.spec --noconfirm
a = Analysis(
    ['../bootstrap/oracle.py'],
    pathex=['..'],
    binaries=[],
    datas=[('../apps/genomelab/genomelab.op', 'apps'), ('../std', 'std')],
    hiddenimports=[],
    runtime_tmpdir=None,
)
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, a.binaries, a.datas, name='GenomeLab-Operon', console=True)
