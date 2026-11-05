"""Gate for a fine-tuned model. Usage: gate.py OLD.lnm NEW.lnm MIX.tsv  (exit 0 = accept)
(1) held-out line CER (held_cer.py logic) must not get worse; (2) magazine ALL12 CER (eval_real.py)
must not get worse by more than 0.15 points."""
import os, re, subprocess, sys
old, new, mix = sys.argv[1:4]
here = os.path.dirname(os.path.abspath(__file__))
env = {**os.environ, "REAL": mix}
def lines(m):
    o = subprocess.run([sys.executable, "held_cer.py", m], cwd=here, env=env, capture_output=True, text=True).stdout
    return float(re.search(r"lines CER ([0-9.]+)", o).group(1))
def mag(m):
    o = subprocess.run([sys.executable, "eval_real.py", m], cwd=here, capture_output=True, text=True).stdout
    return float(re.search(r"ALL12 CER ([0-9.]+)%", o).group(1))
lo, ln = lines(old), lines(new)
mo, mn = mag(old), mag(new)
print(f"held-out lines CER {lo:.4f} -> {ln:.4f};  magazine ALL12 {mo:.2f}% -> {mn:.2f}%")
ok = ln <= lo and mn <= mo + 0.15
print("GATE:", "accept" if ok else "reject")
sys.exit(0 if ok else 1)
