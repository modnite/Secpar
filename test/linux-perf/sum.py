#!/usr/bin/python3
"""Summarises a scenario output file: one line per phase (a phase starts at every `A` line of perfclient)."""
import re, sys, statistics as st

def main(path):
    phase, rows, order = "start", {}, []
    last_s = None
    for line in open(path, errors="replace"):
        line = line.rstrip()
        if line.startswith("A "):
            phase = line.split(" ", 2)[2] if line.count(" ") >= 2 else line
            continue
        if line.startswith("S "):
            f = dict(re.findall(r"(\w+)=([^\s]+)", line))
            key = (phase, f.get("size"))
            if key not in rows:
                rows[key] = {"fps": [], "mbps": [], "age": [], "agemax": [], "host": [], "kwin": []}
                order.append(key)
            r = rows[key]
            r["fps"].append(float(f["fps"])); r["mbps"].append(float(f["mbps"]))
            r["age"].append(float(f["age_avg"][:-2])); r["agemax"].append(float(f["age_max"][:-2]))
            last_s = r
        elif line.startswith("C ") and last_s is not None:
            f = dict(re.findall(r"(\w+)=(\d+)%?", line))
            last_s["host"].append(int(f["host"])); last_s["kwin"].append(int(f["kwin"]))
        elif line.startswith(("P ", "PSUM", "E ", "HOST", "host:")) or "panicked" in line:
            print(line)
    for key in order:
        r = rows[key]
        n = len(r["fps"])
        if n < 2:
            continue
        avg = lambda xs: st.mean(xs) if xs else float("nan")
        print(f"{key[0]:<22} {key[1]:<10} n={n:<3} fps={avg(r['fps']):5.1f} mbps={avg(r['mbps']):5.2f} age_avg={avg(r['age']):5.0f}ms age_max={max(r['agemax']):5.0f}ms host_cpu={avg(r['host']):4.0f}% kwin_cpu={avg(r['kwin']):4.0f}%")

main(sys.argv[1])
