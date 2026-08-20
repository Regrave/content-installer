//! Wings install scripts for modpack installation.

use wings_api::InstallationScript;

/// Non-bash entrypoint: Wings runs `/bin/bash /path/script`.
const CONTAINER_IMAGE: &str = "python:3.12-slim";
const ENTRYPOINT: &str = "/bin/bash";

fn bash_wrapper(python: &str) -> String {
    format!(
        "#!/bin/bash\nset -e\npython3 - <<'CI_INSTALLER_PYTHON'\n{}\nCI_INSTALLER_PYTHON\n",
        python
    )
}

/// Build the install script for a Modrinth `.mrpack` install.
pub fn modrinth_script(
    mrpack_url: &str,
    modpack_name: &str,
    version_name: &str,
) -> InstallationScript {
    let mut environment = indexmap::IndexMap::new();
    environment.insert(
        compact_str::CompactString::from("MRPACK_URL"),
        serde_json::Value::String(mrpack_url.to_string()),
    );
    environment.insert(
        compact_str::CompactString::from("MODPACK_NAME"),
        serde_json::Value::String(modpack_name.to_string()),
    );
    environment.insert(
        compact_str::CompactString::from("MODPACK_VERSION"),
        serde_json::Value::String(version_name.to_string()),
    );

    let script = format!("{}\n{}", PYTHON_COMMON, MODRINTH_PYTHON);

    InstallationScript {
        container_image: compact_str::CompactString::from(CONTAINER_IMAGE),
        entrypoint: compact_str::CompactString::from(ENTRYPOINT),
        script: compact_str::CompactString::from(bash_wrapper(&script)),
        environment,
    }
}

/// Build the install script for a CurseForge modpack install.
pub fn curseforge_script(
    zip_url: &str,
    cf_api_key: &str,
    modpack_name: &str,
    version_name: &str,
) -> InstallationScript {
    let mut environment = indexmap::IndexMap::new();
    environment.insert(
        compact_str::CompactString::from("CF_ZIP_URL"),
        serde_json::Value::String(zip_url.to_string()),
    );
    environment.insert(
        compact_str::CompactString::from("CF_API_KEY"),
        serde_json::Value::String(cf_api_key.to_string()),
    );
    environment.insert(
        compact_str::CompactString::from("MODPACK_NAME"),
        serde_json::Value::String(modpack_name.to_string()),
    );
    environment.insert(
        compact_str::CompactString::from("MODPACK_VERSION"),
        serde_json::Value::String(version_name.to_string()),
    );

    let script = format!("{}\n{}", PYTHON_COMMON, CURSEFORGE_PYTHON);

    InstallationScript {
        container_image: compact_str::CompactString::from(CONTAINER_IMAGE),
        entrypoint: compact_str::CompactString::from(ENTRYPOINT),
        script: compact_str::CompactString::from(bash_wrapper(&script)),
        environment,
    }
}

const PYTHON_COMMON: &str = r###"import datetime, hashlib, json, os, re, shutil, sys, time, tomllib, urllib.request, zipfile
from pathlib import Path

WORKSPACE = Path("/mnt/server")
MODPACK_NAME = os.environ.get("MODPACK_NAME", "Modpack")
MODPACK_VERSION = os.environ.get("MODPACK_VERSION", "Selected version")

RETRYABLE = {408, 425, 429, 500, 502, 503, 504}
PROTECTED = (
    "world",
    "world_nether",
    "world_the_end",
    "server.properties",
    "whitelist.json",
    "banned-ips.json",
    "banned-players.json",
    "ops.json",
    "eula.txt",
    ".mcvc-type.json",
    ".content-installer.log",
)

LOG_PATH = WORKSPACE / ".content-installer.log"


def log(msg):
    line = f"[content-installer] {msg}"
    print(line, flush=True)
    try:
        with open(LOG_PATH, "a", encoding="utf-8") as f:
            f.write(line + "\n")
    except Exception:  # noqa: BLE001
        pass


def reset_log():
    try:
        LOG_PATH.unlink(missing_ok=True)
    except Exception:  # noqa: BLE001
        pass


REMOVED_CLIENT_ONLY = []


def log_summary(name, version, mc, loader_type, total, downloaded, failures):
    log("--- install summary ---")
    log(f"modpack: {name} {version}")
    log(f"minecraft: {mc} | loader: {loader_type}")
    log(f"downloads: {downloaded}/{total} succeeded, {len(failures)} skipped/failed")
    for item in failures:
        log(f"not installed: {item}")
    log(f"client-only mods removed ({len(REMOVED_CLIENT_ONLY)}):")
    for item in REMOVED_CLIENT_ONLY:
        log(f"removed: {item}")


def download(url, dest, headers=None):
    last = None
    for attempt in range(1, 8):
        try:
            req = urllib.request.Request(url, headers=headers or {})
            with urllib.request.urlopen(req, timeout=120) as resp, open(dest, "wb") as out:
                shutil.copyfileobj(resp, out)
            return
        except Exception as e:  # noqa: BLE001
            last = e
            code = getattr(e, "code", None)
            text = str(e).lower()
            retryable = code in RETRYABLE or any(
                k in text
                for k in (
                    "timed out",
                    "timeout",
                    "connection reset",
                    "connection refused",
                    "connection closed",
                    "temporarily unavailable",
                )
            )
            if attempt == 7 or not retryable:
                raise
            delay = 15 if code == 429 else min(2 * (2 ** min(attempt - 1, 3)), 120)
            log(f"download failed ({last}), retrying in {delay}s ({attempt}/7)")
            time.sleep(delay)
    raise last


def get_json(url, headers=None):
    req = urllib.request.Request(url, headers=headers or {})
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.load(resp)


def verify_hash(path, algorithm, expected):
    digest = hashlib.new(algorithm)
    with open(path, "rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    if digest.hexdigest().lower() != str(expected).lower():
        path.unlink(missing_ok=True)
        raise RuntimeError(f"{algorithm} mismatch for {path.name}")


def verify_modrinth_hash(path, hashes):
    for algorithm in ("sha512", "sha1"):
        if hashes.get(algorithm):
            verify_hash(path, algorithm, hashes[algorithm])
            return


def verify_curseforge_hash(path, hashes):
    algorithms = {1: "sha1", 2: "md5"}
    for entry in hashes or []:
        algorithm = algorithms.get(entry.get("algo"))
        if algorithm and entry.get("value"):
            verify_hash(path, algorithm, entry["value"])
            return


def is_protected(rel):
    norm = rel.lstrip("/")
    return any(norm == p or norm.startswith(p + "/") for p in PROTECTED)


def is_safe(path):
    if not path or path.startswith(("/", "\\")):
        return False
    if len(path) >= 2 and path[1] == ":":
        return False
    return ".." not in path.replace("\\", "/").split("/")


def extract_safely(z, dest):
    base = os.path.realpath(dest)
    for info in z.infolist():
        name = (info.filename or "").replace("\\", "/")
        if not is_safe(name):
            log(f"skipping unsafe archive path {name}")
            continue
        if is_protected(name):
            log(f"skipping protected archive path {name}")
            continue
        target = os.path.realpath(os.path.join(base, *name.split("/")))
        if target != base and not target.startswith(base + os.sep):
            log(f"skipping archive path escaping workspace {name}")
            continue
        if info.is_dir():
            os.makedirs(target, exist_ok=True)
            continue
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with z.open(info) as src, open(target, "wb") as dst:
            shutil.copyfileobj(src, dst)


def fetch_exclusions():
    fallback = [
        "optifine", "sodium", "iris", "oculus", "rubidium", "embeddium",
        "entityculling", "fpsreducer", "skinlayers3d", "notenoughanimations",
        "ambientsounds", "fancymenu", "drippyloadingscreen", "blur",
        "modmenu", "controlling", "betterf3", "mousetweaks", "freecam",
        "litematica", "minihud", "tweakeroo", "citresewn", "continuity",
        "chatheads", "reauth", "physicsmod", "roughlyenoughitems", "legendarytooltips",
        "betterthirdperson", "dynamiclights", "ryoamiclights", "immediatelyfast", "reforgium",
    ]
    try:
        data = get_json("https://raw.githubusercontent.com/regrave/content-installer/main/client-only-mods.json")
        return [str(x) for x in data.get("excludes", fallback)]
    except Exception as e:  # noqa: BLE001
        log(f"failed to fetch exclusion list ({e}), using fallback")
        return fallback


def known_client_only(filename, exclusions):
    name = filename.rsplit("/", 1)[-1]
    for suffix in (".jar", ".zip"):
        if name.endswith(suffix):
            name = name[: -len(suffix)]
            break
    name = name.lower()
    return any(
        name.startswith(p) or ("-" + p) in name or ("_" + p) in name
        for p in (str(x).lower() for x in exclusions)
    )


def jar_client_only(jar_path):
    try:
        with zipfile.ZipFile(jar_path) as z:
            names = z.namelist()
            if "fabric.mod.json" in names:
                try:
                    data = json.loads(z.read("fabric.mod.json").decode("utf-8", "replace"))
                    if data.get("environment") == "client":
                        return True
                except Exception:  # noqa: BLE001
                    pass
            if "quilt.mod.json" in names:
                try:
                    data = json.loads(z.read("quilt.mod.json").decode("utf-8", "replace"))
                    if (data.get("quilt_loader") or {}).get("environment") == "client":
                        return True
                except Exception:  # noqa: BLE001
                    pass
            # Forge/NeoForge `side` values belong to individual dependency
            # declarations, not to the containing mod. Likewise, displayTest
            # controls network version compatibility and is not proof that a
            # mod is client-only. Treating either as an environment marker can
            # remove perfectly valid server mods, so use the curated filename
            # list for Forge-family jars instead.
    except Exception:  # noqa: BLE001
        pass
    return False


def mod_ids(jar_path):
    try:
        with zipfile.ZipFile(jar_path) as z:
            names = z.namelist()
            if "fabric.mod.json" in names:
                value = str(
                    json.loads(z.read("fabric.mod.json").decode("utf-8", "replace")).get("id", "")
                ).lower()
                return {value} if value else set()
            if "quilt.mod.json" in names:
                value = str(
                    json.loads(z.read("quilt.mod.json").decode("utf-8", "replace"))
                    .get("quilt_loader", {})
                    .get("id", "")
                ).lower()
                return {value} if value else set()
            for meta in ("META-INF/mods.toml", "META-INF/neoforge.mods.toml"):
                if meta in names:
                    text = z.read(meta).decode("utf-8", "replace")
                    try:
                        data = tomllib.loads(text)
                        return {
                            str(entry.get("modId", "")).lower()
                            for entry in data.get("mods", [])
                            if entry.get("modId")
                        }
                    except Exception:  # noqa: BLE001
                        match = re.search(r'(?m)^\s*modId\s*=\s*"([^"]+)"', text)
                        return {match.group(1).lower()} if match else set()
    except Exception:  # noqa: BLE001
        pass
    return set()


def jar_required_dep_ids(jar):
    required = set()
    try:
        with zipfile.ZipFile(jar) as z:
            names = z.namelist()
            if "fabric.mod.json" in names:
                data = json.loads(z.read("fabric.mod.json").decode("utf-8", "replace"))
                required.update(str(dep).lower() for dep in (data.get("depends") or {}))
            if "quilt.mod.json" in names:
                data = json.loads(z.read("quilt.mod.json").decode("utf-8", "replace"))
                quilt_depends = (data.get("quilt_loader") or {}).get("depends") or []
                if isinstance(quilt_depends, dict):
                    required.update(str(dep).lower() for dep in quilt_depends)
                else:
                    for dep in quilt_depends:
                        if isinstance(dep, str):
                            required.add(dep.lower())
                        elif isinstance(dep, dict) and dep.get("id"):
                            required.add(str(dep["id"]).lower())
            for meta in ("META-INF/mods.toml", "META-INF/neoforge.mods.toml"):
                if meta not in names:
                    continue
                data = tomllib.loads(z.read(meta).decode("utf-8", "replace"))
                for entries in (data.get("dependencies") or {}).values():
                    if isinstance(entries, dict):
                        entries = [entries]
                    for dep in entries or []:
                        dep_type = str(dep.get("type", "required")).lower()
                        if dep.get("mandatory", True) is False or dep_type in {
                            "optional", "incompatible", "discouraged",
                        }:
                            continue
                        if dep.get("modId"):
                            required.add(str(dep["modId"]).lower())
    except Exception:  # noqa: BLE001
        pass
    return required


def required_dep_ids(jars):
    required = set()
    for jar in jars:
        required.update(jar_required_dep_ids(jar))
    return required


def mcjars_zip(kind, mc, requested):
    data = get_json(f"https://versions.mcjars.app/api/v2/builds/{kind}/{mc}")
    builds = data.get("builds") or []
    if not builds:
        raise RuntimeError(f"no {kind} builds available for Minecraft {mc}")
    exact = next(
        (
            build
            for build in builds
            if str(build.get("projectVersionId", "")) == requested
            or str(build.get("name", "")) == requested
        ),
        None,
    )
    if exact and exact.get("zipUrl"):
        return exact["zipUrl"]
    log(f"exact {kind} loader {requested} is unavailable; using {builds[0].get('name', 'latest')}")
    return builds[0]["zipUrl"]


def apply_overrides(src_dir):
    src = (WORKSPACE / src_dir).resolve()
    base = WORKSPACE.resolve()
    if not (src == base or str(src).startswith(str(base) + os.sep)):
        log(f"skipping override dir outside workspace: {src_dir}")
        return
    if not src.exists():
        return
    for entry in sorted(src.iterdir()):
        name = entry.name
        if is_protected(name):
            log(f"skipping protected override {name}")
            continue
        dst = WORKSPACE / name
        if entry.is_dir() and dst.is_dir():
            shutil.copytree(entry, dst, dirs_exist_ok=True)
            shutil.rmtree(entry)
        else:
            if dst.exists():
                if dst.is_dir():
                    shutil.rmtree(dst)
                else:
                    dst.unlink()
            shutil.move(str(entry), str(dst))
        log(f"applied override {name}")


def install_loader(url, is_zip, ltype):
    if is_zip:
        download(url, WORKSPACE / "_loader_install.zip")
        with zipfile.ZipFile(WORKSPACE / "_loader_install.zip") as z:
            extract_safely(z, WORKSPACE)
        (WORKSPACE / "_loader_install.zip").unlink(missing_ok=True)
    else:
        download(url, WORKSPACE / "server.jar")
    return ltype


def resolve_loader(deps, mc):
    if "fabric-loader" in deps:
        return (
            f"https://meta.fabricmc.net/v2/versions/loader/{mc}/{deps['fabric-loader']}/1.0.1/server/jar",
            False,
            "FABRIC",
        )
    if "quilt-loader" in deps:
        return (
            f"https://meta.quiltmc.org/v3/versions/loader/{mc}/{deps['quilt-loader']}/0.10.3/server/jar",
            False,
            "QUILT",
        )
    if "neoforge" in deps:
        return (mcjars_zip("NEOFORGE", mc, str(deps["neoforge"])), True, "NEOFORGE")
    if "forge" in deps:
        return (mcjars_zip("FORGE", mc, str(deps["forge"])), True, "FORGE")
    return None


def remove_client_only_mods():
    mods_dir = WORKSPACE / "mods"
    if not mods_dir.exists():
        return
    exclusions = fetch_exclusions()
    jars = sorted(mods_dir.glob("*.jar"))
    candidates = {}
    for jar in jars:
        if jar_client_only(jar):
            candidates[jar] = "jar scan"
        elif known_client_only(jar.name, exclusions):
            candidates[jar] = "name list"

    # Seed the dependency graph only from mods that will actually remain on
    # the server. A client-only parent must not save its own client-only
    # dependency (for example Sodium Extra causing Sodium to be retained).
    required = required_dep_ids(jar for jar in jars if jar not in candidates)
    changed = True
    while changed:
        changed = False
        for jar in list(candidates):
            if mod_ids(jar) & required:
                log(f"keeping {jar.name}: required dependency")
                required.update(jar_required_dep_ids(jar))
                del candidates[jar]
                changed = True

    for jar, reason in candidates.items():
        log(f"removing client-only mod {jar.name} (caught by {reason})")
        jar.unlink(missing_ok=True)
        REMOVED_CLIENT_ONLY.append(jar.name)


def write_marker(loader_type, mc, modpack_name, extra=None):
    marker = {
        "type": loader_type,
        "version": mc,
        "modpack": modpack_name,
        "installedAt": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    }
    if extra:
        marker.update(extra)
    (WORKSPACE / "eula.txt").write_text("eula=true\n", encoding="utf-8")
    (WORKSPACE / ".mcvc-type.json").write_text(json.dumps(marker), encoding="utf-8")
"###;

// Modrinth-specific body: env var, host allowlist, and main().
const MODRINTH_PYTHON: &str = r###"from urllib.parse import urlparse

MRPACK_URL = os.environ["MRPACK_URL"]
ALLOWED_HOSTS = (
    "cdn.modrinth.com",
    "cdn-raw.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
    "objects.githubusercontent.com",
)


def allowed_url(url):
    host = (urlparse(url).hostname or "").lower()
    return any(host == d or host.endswith("." + d) for d in ALLOWED_HOSTS)


def main():
    reset_log()
    log(f"starting {MODPACK_NAME} ({MODPACK_VERSION})")
    log(f"downloading modpack from {MRPACK_URL}")
    if not allowed_url(MRPACK_URL):
        raise RuntimeError(f"modpack URL host not in allowlist: {MRPACK_URL}")
    download(MRPACK_URL, WORKSPACE / "_mrpack_install.zip")

    log("extracting modpack")
    tmp = WORKSPACE / "_mrpack_temp"
    shutil.rmtree(tmp, ignore_errors=True)
    with zipfile.ZipFile(WORKSPACE / "_mrpack_install.zip") as z:
        extract_safely(z, tmp)

    with open(tmp / "modrinth.index.json", encoding="utf-8") as f:
        index = json.load(f)

    log("applying config overrides")
    apply_overrides("_mrpack_temp/overrides")
    apply_overrides("_mrpack_temp/server-overrides")

    log("checking mod compatibility")
    (WORKSPACE / "mods").mkdir(parents=True, exist_ok=True)

    files = [f for f in index.get("files", []) if (f.get("env") or {}).get("server") != "unsupported"]
    total = len(files)
    skipped = 0
    failures = []
    for i, f in enumerate(files, 1):
        path = f.get("path") or ""
        if not is_safe(path):
            log(f"skipping invalid path {path}")
            skipped += 1
            failures.append(f"{path}: invalid path")
            continue
        if is_protected(path):
            log(f"skipping protected path {path}")
            skipped += 1
            failures.append(f"{path}: protected path")
            continue
        url = next((u for u in f.get("downloads", []) if allowed_url(u)), None)
        if not url:
            raise RuntimeError(f"no allowed download URL for required file {path}")
        dest = WORKSPACE / path
        dest.parent.mkdir(parents=True, exist_ok=True)
        log(f"downloading {i}/{total}: {path}")
        download(url, dest)
        verify_modrinth_hash(dest, f.get("hashes") or {})

    deps = index.get("dependencies") or {}
    mc = str(deps.get("minecraft", "1.21.1"))
    loader = resolve_loader(deps, mc)

    loader_type = "UNKNOWN"
    if loader:
        log(f"installing {loader[2]} loader")
        loader_type = install_loader(*loader)

    log("scanning for client-only mods")
    remove_client_only_mods()

    write_marker(loader_type, mc, index.get("name", ""))

    log("cleaning up")
    shutil.rmtree(tmp, ignore_errors=True)
    (WORKSPACE / "_mrpack_install.zip").unlink(missing_ok=True)
    log(f"modpack installation complete ({total} files, {skipped} skipped)")
    log_summary(index.get("name", ""), index.get("version", ""), mc, loader_type, total, total - skipped, failures)


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # noqa: BLE001
        print(f"[content-installer] install failed: {e}", flush=True)
        sys.exit(1)
"###;

// CurseForge-specific body: env vars, CF API client, and main().
const CURSEFORGE_PYTHON: &str = r###"from urllib.parse import urlparse

CF_ZIP_URL = os.environ["CF_ZIP_URL"]
CF_API_KEY = os.environ.get("CF_API_KEY", "")
CF_ALLOWED_HOSTS = (
    "edge.forgecdn.net",
    "mediafilez.forgecdn.net",
    "media.forgecdn.net",
)
MODRINTH_ALLOWED_HOSTS = (
    "cdn.modrinth.com",
    "cdn-raw.modrinth.com",
)


def allowed_cf_url(url):
    return (urlparse(url).hostname or "").lower() in CF_ALLOWED_HOSTS


def allowed_modrinth_url(url):
    return (urlparse(url).hostname or "").lower() in MODRINTH_ALLOWED_HOSTS


def post_json(url, body, headers=None):
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(
        url, data=data, headers={**({"x-api-key": CF_API_KEY, "Accept": "application/json"} if headers is None else headers), "Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.load(resp)


# CurseForge bills per API call, so resolve file metadata in batches instead of
# one GET per manifest entry. POST /v1/mods/files accepts up to 50 fileIds.
def get_cf_files(file_ids):
    if not CF_API_KEY:
        raise RuntimeError("CurseForge API key not configured")
    ids = [fid for fid in file_ids if fid]
    out = {}
    for i in range(0, len(ids), 50):
        chunk = ids[i:i + 50]
        data = post_json(
            "https://api.curseforge.com/v1/mods/files",
            {"fileIds": chunk},
            {"x-api-key": CF_API_KEY, "Accept": "application/json"},
        )
        for entry in data.get("data", []):
            out[entry.get("id")] = entry
    return out


def curseforge_sha1(hashes):
    return next(
        (
            str(entry.get("value", "")).lower()
            for entry in hashes or []
            if entry.get("algo") == 1 and entry.get("value")
        ),
        None,
    )


# A project author can disable third-party downloads on CurseForge. When that
# happens, look up the trusted CurseForge SHA-1 on Modrinth and use the file only
# if Modrinth independently hosts those exact bytes. This respects CurseForge's
# restriction and avoids scraping or guessing a forbidden ForgeCDN URL.
def get_modrinth_fallbacks(files):
    hashes = sorted(
        {
            sha1
            for file_info in files
            if not file_info.get("downloadUrl")
            if (sha1 := curseforge_sha1(file_info.get("hashes")))
        }
    )
    out = {}
    for i in range(0, len(hashes), 100):
        chunk = hashes[i:i + 100]
        try:
            versions = post_json(
                "https://api.modrinth.com/v2/version_files",
                {"hashes": chunk, "algorithm": "sha1"},
                {
                    "Accept": "application/json",
                    "User-Agent": "Regrave/content-installer/2.8.0",
                },
            )
        except Exception as e:  # noqa: BLE001
            log(f"Modrinth fallback lookup failed ({e}); manual download may be required")
            continue
        if not isinstance(versions, dict):
            continue
        for sha1 in chunk:
            version = versions.get(sha1) or versions.get(sha1.upper()) or {}
            for candidate in version.get("files") or []:
                candidate_hash = str((candidate.get("hashes") or {}).get("sha1", "")).lower()
                url = candidate.get("url")
                if candidate_hash == sha1 and url and allowed_modrinth_url(url):
                    out[sha1] = url
                    break
    return out


def safe_filename(fn):
    fn = fn.replace("/", "").replace("\\", "").replace("..", "")
    return fn or "unknown.jar"


def main():
    if not CF_API_KEY:
        raise RuntimeError("CurseForge API key not configured")
    if not allowed_cf_url(CF_ZIP_URL):
        raise RuntimeError(f"modpack URL host not in allowlist: {CF_ZIP_URL}")
    reset_log()

    log(f"starting {MODPACK_NAME} ({MODPACK_VERSION})")
    log(f"downloading modpack from {CF_ZIP_URL}")
    download(CF_ZIP_URL, WORKSPACE / "_cf_modpack.zip")

    log("extracting modpack")
    tmp = WORKSPACE / "_cf_temp"
    shutil.rmtree(tmp, ignore_errors=True)
    with zipfile.ZipFile(WORKSPACE / "_cf_modpack.zip") as z:
        extract_safely(z, tmp)

    with open(tmp / "manifest.json", encoding="utf-8") as f:
        manifest = json.load(f)

    log("applying config overrides")
    apply_overrides("_cf_temp/" + str(manifest.get("overrides", "overrides")))

    log("checking mod compatibility")
    (WORKSPACE / "mods").mkdir(parents=True, exist_ok=True)

    required = [f for f in manifest.get("files", []) if f.get("required", True)]
    total = len(required)
    downloaded = 0
    skipped = 0
    failures = []
    files_by_id = get_cf_files([cf_file.get("fileID") for cf_file in required])
    modrinth_fallbacks = get_modrinth_fallbacks(files_by_id.values())
    for cf_file in required:
        fid = cf_file.get("fileID")
        file_info = files_by_id.get(fid) or {}
        if not file_info:
            raise RuntimeError(f"failed to resolve required CurseForge file {fid}")
        filename = safe_filename(str(file_info.get("fileName") or "unknown.jar"))
        hashes = file_info.get("hashes")
        destination = WORKSPACE / "mods" / filename

        # Keep valid files from an interrupted install, and make the documented
        # manual-upload recovery usable when clean install is disabled.
        if destination.is_file() and any(
            entry.get("algo") in (1, 2) and entry.get("value") for entry in hashes or []
        ):
            try:
                verify_curseforge_hash(destination, hashes)
                log(f"using existing verified file ({downloaded + 1}/{total}): {filename}")
                downloaded += 1
                continue
            except RuntimeError:
                log(f"existing file failed verification; downloading again: {filename}")

        url = file_info.get("downloadUrl")
        if not url:
            sha1 = curseforge_sha1(hashes)
            url = modrinth_fallbacks.get(sha1)
            if not url:
                log(f"skipping restricted file {filename}: blocks third-party downloads and "
                    "no identical Modrinth file was found; upload it manually to "
                    f"mods/{filename} then reinstall with Clean install off")
                skipped += 1
                project_id = file_info.get("projectId")
                failures.append(
                    f"{filename}: restricted on CurseForge, not on Modrinth; upload to mods/ manually"
                    + (f" (https://www.curseforge.com/minecraft/mc-mods/{project_id}/files/{fid})" if project_id else "")
                )
                continue
            log(f"using identical Modrinth copy for restricted CurseForge file: {filename}")
        elif not allowed_cf_url(url):
            raise RuntimeError(f"required file {filename} returned an untrusted download host")
        log(f"downloading ({downloaded + 1}/{total}): {filename}")
        download(url, destination)
        verify_curseforge_hash(destination, hashes)
        downloaded += 1

    mc = str(manifest.get("minecraft", {}).get("version", "1.21.1"))
    loaders = manifest.get("minecraft", {}).get("modLoaders") or []
    primary = next((l for l in loaders if l.get("primary")), loaders[0] if loaders else None)
    loader_id = str((primary or {}).get("id", ""))
    deps = {}
    for prefix, key in (
        ("forge-", "forge"),
        ("neoforge-", "neoforge"),
        ("fabric-", "fabric-loader"),
        ("quilt-", "quilt-loader"),
    ):
        if loader_id.startswith(prefix):
            deps[key] = loader_id[len(prefix):]
            break

    loader = resolve_loader(deps, mc)

    loader_type = "UNKNOWN"
    if loader:
        log(f"installing {loader[2]} loader")
        loader_type = install_loader(*loader)

    log("scanning for client-only mods")
    remove_client_only_mods()

    write_marker(loader_type, mc, manifest.get("name", ""), extra={"source": "curseforge"})

    log("cleaning up")
    shutil.rmtree(tmp, ignore_errors=True)
    (WORKSPACE / "_cf_modpack.zip").unlink(missing_ok=True)
    log(f"modpack installation complete ({downloaded} files, {skipped} skipped)")
    log_summary(manifest.get("name", ""), manifest.get("version", ""), mc, loader_type, total, downloaded, failures)


if __name__ == "__main__":
    try:
        main()
    except Exception as e:  # noqa: BLE001
        print(f"[content-installer] install failed: {e}", flush=True)
        sys.exit(1)
"###;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, process::Stdio};

    fn assert_python_compiles(source: &str) {
        let mut child = match std::process::Command::new("python3")
            .args(["-c", "import sys; compile(sys.stdin.read(), '<installer>', 'exec')"])
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("failed to start python3: {error}"),
        };

        child
            .stdin
            .take()
            .expect("python stdin")
            .write_all(source.as_bytes())
            .expect("write Python source");
        assert!(child.wait().expect("wait for python3").success());
    }

    #[test]
    fn embedded_python_is_syntactically_valid() {
        assert_python_compiles(&format!("{PYTHON_COMMON}\n{MODRINTH_PYTHON}"));
        assert_python_compiles(&format!("{PYTHON_COMMON}\n{CURSEFORGE_PYTHON}"));
    }

    #[test]
    fn scripts_keep_secrets_in_the_environment() {
        let script = curseforge_script(
            "https://edge.forgecdn.net/files/1/pack.zip",
            "secret-key",
            "Example Pack",
            "1.0.0",
        );

        assert!(!script.script.contains("secret-key"));
        assert_eq!(
            script.environment.get("CF_API_KEY"),
            Some(&serde_json::Value::String("secret-key".to_string()))
        );
        assert_eq!(
            script.environment.get("MODPACK_NAME"),
            Some(&serde_json::Value::String("Example Pack".to_string()))
        );
    }

    #[test]
    fn generated_scripts_use_the_native_install_contract() {
        let script = modrinth_script(
            "https://cdn.modrinth.com/data/example/pack.mrpack",
            "Example Pack",
            "1.0.0",
        );

        assert_eq!(script.container_image.as_str(), CONTAINER_IMAGE);
        assert_eq!(script.entrypoint.as_str(), ENTRYPOINT);
        assert!(script.script.starts_with("#!/bin/bash\nset -e\n"));
        assert!(script.script.contains("CI_INSTALLER_PYTHON"));
    }

    #[test]
    fn curseforge_installer_batches_and_applies_a_complete_manifest() {
        let fixture = r###"
import tempfile

calls = []
modrinth_calls = []
downloaded_urls = []


def post_json(url, body, headers=None):
    if url == "https://api.curseforge.com/v1/mods/files":
        assert headers == {"x-api-key": "test-key", "Accept": "application/json"}
        file_ids = body["fileIds"]
        calls.append(list(file_ids))
        return {
            "data": [
                {
                    "id": file_id,
                    "fileName": f"fixture-{file_id}.jar",
                    "downloadUrl": (
                        None
                        if file_id == 51
                        else f"https://edge.forgecdn.net/files/test/{file_id}.jar"
                    ),
                    "hashes": (
                        [{
                            "algo": 1,
                            "value": hashlib.sha1(f"fixture-{file_id}".encode()).hexdigest(),
                        }]
                        if file_id in (51, 52)
                        else []
                    ),
                }
                for file_id in file_ids
            ]
        }

    assert url == "https://api.modrinth.com/v2/version_files"
    assert headers == {
        "Accept": "application/json",
        "User-Agent": "Regrave/content-installer/2.8.0",
    }
    assert body["algorithm"] == "sha1"
    modrinth_calls.append(list(body["hashes"]))
    return {
        sha1: {
            "files": [{
                "url": f"https://cdn.modrinth.com/data/fixture/{sha1}/fixture-51.jar",
                "hashes": {"sha1": sha1},
            }]
        }
        for sha1 in body["hashes"]
    }


def download(url, destination, headers=None):
    if destination.name == "_cf_modpack.zip":
        manifest = {
            "name": "CurseForge fixture",
            "minecraft": {"version": "1.20.1", "modLoaders": []},
            "files": [
                {"projectID": file_id + 1000, "fileID": file_id, "required": True}
                for file_id in range(1, 122)
            ],
            "overrides": "overrides",
        }
        with zipfile.ZipFile(destination, "w") as archive:
            archive.writestr("manifest.json", json.dumps(manifest))
            archive.writestr("overrides/config/fixture.txt", "fixture config")
            archive.writestr("overrides/server.properties", "must not replace")
        return

    downloaded_urls.append(url)
    if url.startswith("https://cdn.modrinth.com/"):
        destination.write_bytes(b"fixture-51")
        return

    assert url.startswith("https://edge.forgecdn.net/files/test/")
    with zipfile.ZipFile(destination, "w") as archive:
        file_id = destination.stem.removeprefix("fixture-")
        archive.writestr(
            "fabric.mod.json",
            json.dumps({"schemaVersion": 1, "id": f"fixture_{file_id}", "version": "1"}),
        )


fetch_exclusions = lambda: []

with tempfile.TemporaryDirectory(prefix="content-installer-cf-") as root:
    WORKSPACE = Path(root)
    (WORKSPACE / "server.properties").write_text("existing=true\n", encoding="utf-8")
    (WORKSPACE / "mods").mkdir()
    (WORKSPACE / "mods/fixture-52.jar").write_bytes(b"fixture-52")
    main()

    assert [len(call) for call in calls] == [50, 50, 21]
    assert len(modrinth_calls) == 1 and len(modrinth_calls[0]) == 1
    assert any(url.startswith("https://cdn.modrinth.com/") for url in downloaded_urls)
    assert not any(url.endswith("/52.jar") for url in downloaded_urls)
    assert len(list((WORKSPACE / "mods").glob("fixture-*.jar"))) == 121
    assert (WORKSPACE / "config/fixture.txt").read_text() == "fixture config"
    assert (WORKSPACE / "server.properties").read_text() == "existing=true\n"
    marker = json.loads((WORKSPACE / ".mcvc-type.json").read_text())
    assert marker["source"] == "curseforge"
    assert marker["modpack"] == "CurseForge fixture"
    assert marker["version"] == "1.20.1"

    shutil.rmtree(WORKSPACE / "mods")
    (WORKSPACE / "mods").mkdir()

    def fabric_jar(filename, mod_id, environment="*", depends=None):
        with zipfile.ZipFile(WORKSPACE / "mods" / filename, "w") as archive:
            archive.writestr(
                "fabric.mod.json",
                json.dumps({
                    "schemaVersion": 1,
                    "id": mod_id,
                    "version": "1",
                    "environment": environment,
                    "depends": depends or {},
                }),
            )

    fabric_jar("server.jar", "server", depends={"required_client_lib": "*"})
    fabric_jar(
        "required-client-lib.jar",
        "required_client_lib",
        environment="client",
        depends={"nested_required_lib": "*"},
    )
    fabric_jar("nested-required-lib.jar", "nested_required_lib", environment="client")
    fabric_jar(
        "sodium-extra.jar",
        "sodium_extra",
        environment="client",
        depends={"sodium": "*"},
    )
    fabric_jar("sodium.jar", "sodium", environment="client")
    with zipfile.ZipFile(WORKSPACE / "mods/forge-server.jar", "w") as archive:
        archive.writestr(
            "META-INF/mods.toml",
            'displayTest="IGNORE_ALL_VERSION"\n[[mods]]\nmodId="forge_server"\nversion="1"\n'
            '[[dependencies.forge_server]]\nmodId="client_helper"\nmandatory=false\nside="CLIENT"\n',
        )

    remove_client_only_mods()
    remaining = {path.name for path in (WORKSPACE / "mods").glob("*.jar")}
    assert "server.jar" in remaining
    assert "required-client-lib.jar" in remaining
    assert "nested-required-lib.jar" in remaining
    assert "forge-server.jar" in remaining
    assert "sodium-extra.jar" not in remaining
    assert "sodium.jar" not in remaining
"###;

        let source = format!("{PYTHON_COMMON}\n{CURSEFORGE_PYTHON}\n{fixture}");
        let mut child = match std::process::Command::new("python3")
            .args([
                "-c",
                "import sys; exec(compile(sys.stdin.read(), '<installer-test>', 'exec'), {'__name__': 'embedded_test'})",
            ])
            .env("CF_ZIP_URL", "https://edge.forgecdn.net/files/1/pack.zip")
            .env("CF_API_KEY", "test-key")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("failed to start python3: {error}"),
        };

        child
            .stdin
            .take()
            .expect("python stdin")
            .write_all(source.as_bytes())
            .expect("write Python source");
        let output = child.wait_with_output().expect("wait for Python fixture");
        assert!(
            output.status.success(),
            "CurseForge fixture failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
