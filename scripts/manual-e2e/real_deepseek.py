"""Use the player's real DeepSeek key in an isolated test instance — in memory only.

The key is read from the Windows Credential Manager entry that the player's own Kotone wrote
(keyring service `kotone`, user `connection:<id>`; keyring-rs stores it as target `<user>.<service>`),
handed to the isolated instance through stdin of `kotone.exe tools call connections.upsert --args-file -`
(the instance stores it in its own credential space `kotone@<KOTONE_HOME>`), and never printed, logged or
written to a file. Call `delete_connection` when the test is done.

The player's connection id defaults to `deepseek-mwjiotc0`; override with KOTONE_REAL_DEEPSEEK_CONNECTION.
If the credential is missing, set KOTONE_REAL_DEEPSEEK_KEY in the environment instead (e.g. `! $env:...`).
"""
import ctypes
import ctypes.wintypes as wt
import json
import os
import subprocess

CONNECTION_ID = os.environ.get("KOTONE_REAL_DEEPSEEK_CONNECTION", "deepseek-mwjiotc0")
TEST_CONNECTION_ID = "deepseek-real-test"


class _CREDENTIAL(ctypes.Structure):
    _fields_ = [
        ("Flags", wt.DWORD), ("Type", wt.DWORD), ("TargetName", wt.LPWSTR), ("Comment", wt.LPWSTR),
        ("LastWritten", wt.FILETIME), ("CredentialBlobSize", wt.DWORD),
        ("CredentialBlob", ctypes.POINTER(ctypes.c_ubyte)), ("Persist", wt.DWORD),
        ("AttributeCount", wt.DWORD), ("Attributes", ctypes.c_void_p), ("TargetAlias", wt.LPWSTR),
        ("UserName", wt.LPWSTR),
    ]


def read_key():
    """The player's DeepSeek key, or None. Never print the return value."""
    env = os.environ.get("KOTONE_REAL_DEEPSEEK_KEY", "").strip()
    if env:
        return env
    advapi = ctypes.WinDLL("advapi32", use_last_error=True)
    advapi.CredReadW.argtypes = [wt.LPCWSTR, wt.DWORD, wt.DWORD, ctypes.POINTER(ctypes.POINTER(_CREDENTIAL))]
    advapi.CredFree.argtypes = [ctypes.c_void_p]
    pcred = ctypes.POINTER(_CREDENTIAL)()
    target = f"connection:{CONNECTION_ID}.kotone"
    if not advapi.CredReadW(target, 1, 0, ctypes.byref(pcred)):  # CRED_TYPE_GENERIC
        return None
    try:
        size = pcred.contents.CredentialBlobSize
        blob = bytes(pcred.contents.CredentialBlob[:size])
    finally:
        advapi.CredFree(pcred)
    # keyring-rs stores passwords as UTF-16LE.
    try:
        text = blob.decode("utf-16-le")
    except UnicodeDecodeError:
        text = blob.decode("utf-8", errors="ignore")
    return text.strip() or None


def tools_call(exe, env, tool, args, timeout_ms=30_000):
    """`kotone.exe tools call` with the arguments on stdin (nothing on disk, nothing on the command line)."""
    out = subprocess.run([exe, "tools", "call", tool, "--args-file", "-", "--timeout-ms", str(timeout_ms)],
                         input=json.dumps(args, ensure_ascii=False), capture_output=True, text=True,
                         encoding="utf-8", env=env)
    try:
        envelope = json.loads(out.stdout)
    except json.JSONDecodeError:
        raise RuntimeError(f"{tool}: no JSON from the CLI (exit {out.returncode})")
    if not envelope.get("ok"):
        raise RuntimeError(f"{tool}: {json.dumps(envelope.get('error'), ensure_ascii=False)}")
    return envelope["result"]


def add_connection(exe, env, model="deepseek-chat"):
    """Create the test DeepSeek connection in the isolated instance with the real key. Returns its id."""
    key = read_key()
    if not key:
        raise RuntimeError("no DeepSeek key: credential not found and KOTONE_REAL_DEEPSEEK_KEY not set")
    tools_call(exe, env, "connections.upsert", {
        "connection": {"id": TEST_CONNECTION_ID, "displayName": "DeepSeek（真实，测试用）", "kind": "remote",
                       "provider": "deepseek", "baseUrl": "https://api.deepseek.com/v1", "model": model},
        "apiKey": key,
    })
    del key
    return TEST_CONNECTION_ID


def delete_connection(exe, env):
    try:
        tools_call(exe, env, "connections.delete", {"id": TEST_CONNECTION_ID})
    except RuntimeError as e:
        print(f"WARN could not delete the test connection: {e}")


if __name__ == "__main__":
    print("key found" if read_key() else "key NOT found")
