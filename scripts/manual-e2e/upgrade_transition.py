"""0.1.13 -> 0.1.14 -> 0.2.0 settings/profile round trips in disposable data directories.

Pass three source worktrees, not installed apps. This does not run installers, register hotkeys,
send chat, or read the Windows credential vault. See ADR 027 for the separate VM test.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
from zipfile import ZipFile

PROBE = r"""
use kotone_core::{profile, settings};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("isolated home required"));
    let command = args.next().expect("missing probe command");
    let config = path.join("config.json");
    let profiles = path.join("profiles");
    let value = settings::load_from(&config);
    if command == "persist" {
        settings::save_to(&config, &value).unwrap();
    }
    profile::ensure_builtin_in(&profiles).unwrap();
    if command == "delete-dota" {
        profile::delete_profile_in(&profiles, "dota2").unwrap();
        profile::ensure_builtin_in(&profiles).unwrap();
    }
    println!("{}", serde_json::json!({
        "settings": value,
        "profiles": profile::list_in(&profiles).into_iter().map(|profile| profile.id).collect::<Vec<_>>(),
        "dotaMarker": profiles.join(".dota2-deleted").is_file(),
    }));
}
"""

VOICE_FIELDS = (
    "hotkey", "hotkeyBackend", "interactionMode", "activeProfileId",
    "audioDeviceId", "sttEngine", "activeModelId", "language",
    "autoSend", "models", "vad", "vadSilenceMs", "soundFeedback",
    "channelCycleHotkey", "resendLastHotkey",
)
REMINDER_FIELDS = ("hotkey", "interactionMode", "connectionId", "defaultDelivery", "speechEngine")
MODES = {"push-to-talk", "dictation", "one-shot", "solo"}
KEY = "lowerSpeakerVolumeWhileListening"

def json_write(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

def safe_regular(path):
    return path.is_file() and not (path.lstat().st_file_attributes & 0x400)

def diagnostic_seed(archive, home):
    with ZipFile(archive) as package:
        for name in ("manifest.json", "runtime.json"):
            if package.getinfo(name).file_size > 65536:
                raise ValueError("oversized diagnostic summary")
        version = json.loads(package.read("manifest.json"))["appVersion"]
        runtime = json.loads(package.read("runtime.json"))
    if not re.fullmatch(r"0\.1\.\d+", version):
        raise ValueError("diagnostic snapshot is not from 0.1.x")
    mode = runtime.get("interactionMode")
    if mode not in MODES:
        mode = "push-to-talk"
    hotkey = runtime.get("hotkey")
    if not isinstance(hotkey, str) or not re.fullmatch(r"[+\w`~ -]{1,48}", hotkey, flags=re.ASCII):
        hotkey = "Ctrl+Alt+F9"
    config = {
        "ui": {"firstRunCompleted": True},
        "hotkey": {"key": hotkey, "mode": "hold" if mode == "push-to-talk" else "toggle"},
        "interactionMode": mode,
        "activeProfileId": "lol" if runtime.get("activeProfileId") == "lol" else "generic",
        "assistant": {"hotkey": "", "interactionMode": "push-to-talk"},
    }
    engine = runtime.get("engineId")
    model = runtime.get("modelId")
    if isinstance(engine, str) and re.fullmatch(r"[a-z0-9-]{1,80}", engine):
        config["sttEngine"] = engine
    if isinstance(model, str) and re.fullmatch(r"[a-zA-Z0-9_.-]{1,160}", model):
        config["activeModelId"] = model
    json_write(home / "config.json", config)

def supplied_seed(source, home):
    config = source / "config.json"
    if not safe_regular(config) or config.stat().st_size > 2_000_000:
        raise ValueError("sample requires a regular config.json")
    data = json.loads(config.read_text(encoding="utf-8-sig"))
    if not isinstance(data, dict):
        raise ValueError("sample config must contain a JSON object")
    json_write(home / "config.json", data)
    profiles = source / "profiles"
    if profiles.is_dir() and not (profiles.lstat().st_file_attributes & 0x400):
        (home / "profiles").mkdir()
        for candidate in profiles.glob("*.json"):
            if safe_regular(candidate) and candidate.stat().st_size < 2_000_000:
                shutil.copyfile(candidate, home / "profiles" / candidate.name)
    reminder_file = source / "reminders.json"
    if safe_regular(reminder_file) and reminder_file.stat().st_size < 2_000_000:
        shutil.copyfile(reminder_file, home / "reminders.json")

def synthetic_seed(home):
    json_write(home / "config.json", {
        "ui": {"firstRunCompleted": True, "autoStart": False},
        "hotkey": {"key": "Ctrl+Alt+F9", "mode": "hold"},
        "assistant": {"hotkey": "Ctrl+Alt+F10", "interactionMode": "push-to-talk",
                      "defaultDelivery": ["danmaku"], "speechEngine": "system"},
        "interactionMode": "push-to-talk",
        "activeProfileId": "lol",
        "audioDeviceId": "default",
        "language": "zh",
        "sttEngine": "sherpa-streaming",
        "hotwordsScore": 2.5,
    })

def build_probe(root, source, name):
    project = root / name
    (project / "src").mkdir(parents=True)
    core = json.dumps((source / "crates/kotone-core").as_posix())
    (project / "Cargo.toml").write_text(
        '[package]\nname="kotone-transition-probe-' + name + '"\nversion="0.0.0"\nedition="2021"\n'
        '[dependencies]\nserde_json="1"\nkotone-core={path=' + core + '}\n[workspace]\n', encoding="utf-8",
    )
    (project / "src/main.rs").write_text(PROBE, encoding="utf-8")
    env = {**os.environ, "CARGO_TARGET_DIR": str(root / "probe-target"), "CARGO_BUILD_JOBS": "1"}
    subprocess.run(["cargo", "build", "--offline", "--quiet", "--manifest-path", project / "Cargo.toml"],
                   check=True, env=env)
    return root / "probe-target/debug" / f"kotone-transition-probe-{name}.exe"

def read(probe, home, command="persist"):
    out = subprocess.check_output([probe, home, command], text=True, encoding="utf-8",
                                  env={**os.environ, "KOTONE_HOME": str(home)})
    return json.loads(out)

def require_same(a, b):
    for field in VOICE_FIELDS:
        if a.get(field) != b.get(field):
            raise AssertionError(f"voice setting changed: {field}")
    for field in REMINDER_FIELDS:
        if a.get("assistant", {}).get(field) != b.get("assistant", {}).get(field):
            raise AssertionError(f"legacy reminder setting changed: {field}")

def exercise(home, old, maint, new):
    baseline = read(old, home)["settings"]
    maintenance = read(maint, home)
    require_same(baseline, maintenance["settings"])
    assert maintenance["profiles"].count("dota2") == 1
    config = home / "config.json"
    data = json.loads(config.read_text(encoding="utf-8"))
    assert data[KEY] is False
    data[KEY] = True
    json_write(config, data)
    keep = home.parent / (home.name + "-keep")
    shutil.copytree(home, keep)
    d = read(maint, home, "delete-dota")
    assert d["dotaMarker"] and "dota2" not in d["profiles"]

    for case, expected in ((home, 0), (keep, 1)):
        upgraded = read(new, case)
        require_same(baseline, upgraded["settings"])
        assert upgraded["settings"]["ui"]["legacyUpgrade"] == "pending"
        assert upgraded["settings"][KEY] is True
        assert upgraded["profiles"].count("dota2") == expected
        if expected == 1:
            opted_config = case / "config.json"
            opted_data = json.loads(opted_config.read_text(encoding="utf-8"))
            opted_data["ui"]["legacyUpgrade"] = "optedIn"
            opted_data["assistant"]["companionBindings"] = {
                "hotkey": "F22", "connectionId": "isolated-companion-connection",
            }
            json_write(opted_config, opted_data)
            opted = read(new, case)["settings"]
            require_same(baseline, opted)
            assert opted["assistant"]["companionBindings"] == opted_data["assistant"]["companionBindings"]
        downgraded = read(maint, case)
        require_same(baseline, downgraded["settings"])
        assert downgraded["settings"][KEY] is True
        assert downgraded["profiles"].count("dota2") == expected
        require_same(baseline, read(old, case, "inspect")["settings"])

def main():
    if os.name != "nt":
        raise SystemExit("This upgrade matrix targets Windows.")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--old", required=True, type=Path, help="v0.1.13 source worktree")
    parser.add_argument("--maintenance", required=True, type=Path, help="0.1.14 source worktree")
    parser.add_argument("--new", required=True, type=Path, help="0.2.0 source worktree")
    parser.add_argument("--diagnostic", action="append", type=Path, default=[])
    parser.add_argument("--sample", action="append", type=Path, default=[], help="folder with anonymous config.json")
    args = parser.parse_args()
    source = args.new.resolve()
    resource_dir = Path("crates/kotone-core/resources/profiles")
    for asset in (resource_dir / "dota2.json", resource_dir / "icons/dota2-kotone.webp"):
        assert (args.maintenance / asset).read_bytes() == (source / asset).read_bytes(), asset
    test_root = source / "target/upgrade-e2e"
    test_root.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="roundtrip-", dir=test_root))
    probes = tuple(build_probe(root, branch.resolve(), label) for branch, label in
                   ((args.old, "old"), (args.maintenance, "maint"), (args.new, "new")))
    fixtures = [("synthetic", synthetic_seed)]
    fixtures += [(f"diagnostic-{i}", lambda home, f=f: diagnostic_seed(f, home))
                 for i, f in enumerate(args.diagnostic, 1)]
    fixtures += [(f"anonymous-{i}", lambda home, f=f: supplied_seed(f, home))
                 for i, f in enumerate(args.sample, 1)]
    for label, seed in fixtures:
        home = root / label
        home.mkdir()
        seed(home)
        exercise(home, *probes)
        print(f"PASS {label}: versioned settings, Dota 2 removal/uniqueness, volume preference round trip.")
    print(f"PASS {len(fixtures)} fixtures; data isolated in {root}")

if __name__ == "__main__":
    main()
