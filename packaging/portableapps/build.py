#!/usr/bin/env python3
"""Build a PortableApps-layout folder (and zip) from built binaries (roadmap 1.11).

usage: build.py BIN_DIR OUT_DIR VERSION      e.g. build.py target/release dist 0.2.0-pre.10

Layout (PortableApps.com format 3.5):
  LegatorOCRPortable/App/AppInfo/appinfo.ini
  LegatorOCRPortable/App/LegatorOCR/{legatorocr[.exe], legatorocr-gui[.exe], portable.txt}
  LegatorOCRPortable/Data/             (settings go here: the GUI sees portable.txt)
  LegatorOCRPortable/Other/Source/     (licence and source pointer)
  LegatorOCRPortable/Start-LegatorOCR.cmd   (starts the GUI; a stand-in for the .exe launcher)
Not made here: the PortableApps.com Launcher .exe, icons and the installer (need Windows tools).
"""
import os, re, shutil, sys, zipfile

def main(bin_dir, out_dir, version):
    if not re.fullmatch(r"\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?", version):
        sys.exit("bad version")
    nums = re.match(r"(\d+)\.(\d+)\.(\d+)(?:-[A-Za-z]+\.(\d+))?", version).groups()
    pkg_version = ".".join(n or "0" for n in nums)
    root = os.path.join(out_dir, "LegatorOCRPortable")
    shutil.rmtree(root, ignore_errors=True)
    app = os.path.join(root, "App", "LegatorOCR")
    for d in (app, os.path.join(root, "App", "AppInfo"), os.path.join(root, "Data"), os.path.join(root, "Other", "Source")):
        os.makedirs(d)
    found = 0
    for name in ("legatorocr", "legatorocr-gui"):
        for ext in ("", ".exe"):
            p = os.path.join(bin_dir, name + ext)
            if os.path.isfile(p):
                shutil.copy2(p, app); found += 1
    if not found:
        sys.exit(f"no binaries in {bin_dir}")
    open(os.path.join(app, "portable.txt"), "w").write("Settings are kept in ..\\..\\Data while this file is here.\n")
    open(os.path.join(root, "Data", "README.txt"), "w").write("LegatorOCR settings (settings.lgr) are stored here in portable mode.\n")
    shutil.copy2("LICENSE", os.path.join(root, "Other", "Source", "LICENSE.txt"))
    open(os.path.join(root, "Other", "Source", "SOURCE.txt"), "w").write("Source: https://github.com/chiuta/LegatorOCR (tag matching version %s)\n" % version)
    open(os.path.join(root, "App", "AppInfo", "appinfo.ini"), "w").write(f"""[Format]
Type=PortableAppsFormat
Version=3.5

[Details]
Name=LegatorOCR Portable
AppID=LegatorOCRPortable
Publisher=Alexio (TROM) & contributors
Homepage=github.com/chiuta/LegatorOCR
Category=Graphics and Pictures
Description=Scan clean-up, PDF export and OCR
Language=Multilingual
Trademarks=
InstallType=

[License]
Shareable=true
OpenSource=true
Freeware=true
CommercialUse=true

[Version]
PackageVersion={pkg_version}
DisplayVersion={version}

[Control]
Icons=1
Start=Start-LegatorOCR.cmd
""")
    open(os.path.join(root, "Start-LegatorOCR.cmd"), "w", newline="\r\n").write('@echo off\r\nstart "" "%~dp0App\\LegatorOCR\\legatorocr-gui.exe"\r\n')
    zpath = os.path.join(out_dir, f"LegatorOCRPortable-{version}.zip")
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED) as z:
        for dp, _, fs in os.walk(root):
            for f in fs:
                full = os.path.join(dp, f)
                z.write(full, os.path.relpath(full, out_dir))
    print("wrote", zpath)

if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
