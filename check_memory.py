"""Repeated native menu rebuilds on demo data; never connects to a router."""
import re
import os
from pathlib import Path
import subprocess
import tempfile
import time

root = Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(dir=root / 'target') as tmp:
    config = Path(tmp) / 'settings.json'
    config.write_text((root / 'settings.example.json').read_text())
    proc = subprocess.Popen(
        [str(root / 'dist/MikroCockpit.app/Contents/MacOS/MikroCockpit'), '--memory-smoke'],
        env={**os.environ, 'MIKROCOCKPIT_CONFIG': str(config)},
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True,
    )
    samples = []
    try:
        for second in range(1, 20):
            time.sleep(1)
            if proc.poll() is not None:
                break
            rss = int(subprocess.check_output(['ps', '-o', 'rss=', '-p', str(proc.pid)], text=True))
            samples.append(rss)
            if second in (5, 10, 15, 19):
                print(f'{second}s: RSS {rss / 1024:.1f} MiB', flush=True)
            if second == 15:
                report = subprocess.check_output(['vmmap', '-summary', str(proc.pid)], text=True)
                (root / 'target/memory-smoke-vmmap.txt').write_text(report)
        _, err = proc.communicate(timeout=10)
        print(err.strip())
        assert proc.returncode == 0, err
        renders = re.search(r'OK \((\d+) renders\)', err)
        assert renders and int(renders[1]) >= 200, 'Too few actual menu rebuilds'
        assert len(samples) >= 10, 'Too few memory samples'
        growth = max(samples[-5:]) - min(samples[4:9])
        print(f'Growth after warmup: {growth / 1024:.1f} MiB')
        assert growth < 20 * 1024, 'Native menu memory keeps growing'
    finally:
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=5)
