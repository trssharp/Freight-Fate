#!/usr/bin/env python3
"""Find the iOS TestFlight signing profile through the App Store Connect API.

Used by ``.github/workflows/ios-testflight.yml``. The Apple distribution
certificate and its private key are stored once, as the repository secrets
IOS_DISTRIBUTION_P12_BASE64 and IOS_DISTRIBUTION_P12_PASSWORD, and every run
signs with that same certificate. Nothing here creates or revokes a
certificate, so Apple sends no "certificate revoked" mail after a run.

Sub-commands (the API key comes from the environment, never the command line):

    prepare    check the key, find the stored certificate on the team by its
               serial number, find or register the bundle id, then reuse the
               App Store profile that names that certificate (making one only
               when none is active) and write it under --out
    check-app  check that the App Store Connect app record exists
    next-build print the next build number for TestFlight
    distribute add an uploaded build to an external tester group (Friends)
               and submit it to Apple's beta review; internal testers get
               every build without this
    cleanup    delete the local profile and certificate files; nothing in
               App Store Connect is touched

Environment: ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH (the downloaded .p8).

Only the standard library and the ``openssl`` command are used. Nothing in
this file prints a key, a token, a certificate or a profile; error messages
carry Apple's error title and detail and nothing else from a response.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

API = "https://api.appstoreconnect.apple.com"


def _load_build_ios():
    """tools/build_ios.py by path: the one place the bundle id and version are set."""
    import importlib.util
    import sys

    path = Path(__file__).resolve().parent / "build_ios.py"
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("build_ios_for_signing", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


_BUILD_IOS = _load_build_ios()
BUNDLE_ID = _BUILD_IOS.BUNDLE_ID
APP_NAME = "Freight Fate"
CERTIFICATE_TYPES = ("DISTRIBUTION", "IOS_DISTRIBUTION")
PROFILE_TYPE = "IOS_APP_STORE"
TOKEN_LIFETIME = 15 * 60  # Apple allows at most 20 minutes.

CERT_FILE = "distribution.cert.pem"
PROFILE_FILE = "profile.mobileprovision"
P12_FILE = "distribution.p12"


class SigningError(Exception):
    """A failure whose message is safe to print and tells the reader what to do."""


# --- JWT --------------------------------------------------------------------


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def der_to_raw_signature(der: bytes, size: int = 32) -> bytes:
    """Convert an ASN.1 DER ECDSA signature to the fixed-width r||s JWT form."""
    if len(der) < 8 or der[0] != 0x30:
        raise SigningError("openssl returned a signature in an unexpected format.")
    pos = 2
    if der[1] & 0x80:
        pos = 2 + (der[1] & 0x7F)
    parts = []
    for _ in range(2):
        if der[pos] != 0x02:
            raise SigningError("openssl returned a signature in an unexpected format.")
        length = der[pos + 1]
        value = der[pos + 2 : pos + 2 + length]
        parts.append(value.lstrip(b"\x00").rjust(size, b"\x00"))
        pos += 2 + length
    if any(len(p) != size for p in parts):
        raise SigningError("openssl returned a signature of an unexpected size.")
    return b"".join(parts)


def sign_es256(signing_input: bytes, key_path: Path) -> bytes:
    result = subprocess.run(
        ["openssl", "dgst", "-sha256", "-sign", str(key_path)],
        input=signing_input,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise SigningError(
            "openssl could not sign with the App Store Connect key. "
            "Check that ASC_KEY_P8_BASE64 holds the base64 of the downloaded AuthKey .p8 file."
        )
    return der_to_raw_signature(result.stdout)


def make_token(key_id: str, issuer_id: str, key_path: Path, now: float | None = None) -> str:
    issued = int(time.time() if now is None else now)
    header = {"alg": "ES256", "kid": key_id, "typ": "JWT"}
    payload = {
        "iss": issuer_id,
        "iat": issued,
        "exp": issued + TOKEN_LIFETIME,
        "aud": "appstoreconnect-v1",
    }
    signing_input = (
        _b64url(json.dumps(header, separators=(",", ":")).encode())
        + "."
        + _b64url(json.dumps(payload, separators=(",", ":")).encode())
    ).encode("ascii")
    return signing_input.decode("ascii") + "." + _b64url(sign_es256(signing_input, key_path))


# --- HTTP -------------------------------------------------------------------


def _error_text(body: bytes) -> str:
    try:
        errors = json.loads(body).get("errors", [])
    except (ValueError, AttributeError):
        return ""
    lines = []
    for error in errors[:3]:
        title = str(error.get("title", ""))
        detail = str(error.get("detail", ""))
        lines.append(f"{title}: {detail}".strip(": "))
    return "; ".join(lines)


def api_request(token: str, method: str, path: str, body: dict | None = None) -> dict:
    """Call the API; returns the decoded JSON ({} for an empty reply)."""
    url = path if path.startswith("http") else API + path
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(url, data=data, method=method)
    request.add_header("Authorization", f"Bearer {token}")
    if data is not None:
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            raw = response.read()
    except urllib.error.HTTPError as error:
        raise ApiError(method, path, error.code, _error_text(error.read())) from None
    except (urllib.error.URLError, TimeoutError) as error:
        raise SigningError(
            f"Could not reach the App Store Connect API ({type(error).__name__}). Run it again."
        ) from None
    return json.loads(raw) if raw else {}


class ApiError(SigningError):
    def __init__(self, method: str, path: str, status: int, detail: str):
        self.status = status
        self.detail = detail
        self.method = method
        self.path = path.split("?")[0]
        super().__init__(f"{method} {self.path} failed with HTTP {status}. {detail}".strip())


ACCESS_HELP = (
    "The App Store Connect API key was rejected. Check that ASC_KEY_ID, ASC_ISSUER_ID and "
    "ASC_KEY_P8_BASE64 belong to the same key, that the key has not been revoked, and that "
    "its access is App Manager or Admin (Users and Access, Integrations, App Store Connect API)."
)


def explain(error: ApiError) -> SigningError:
    if error.status == 401:
        return SigningError(ACCESS_HELP)
    if error.status == 403:
        return SigningError(
            f"The App Store Connect API key is not allowed to do this ({error.method} "
            f"{error.path}). Give the key App Manager or Admin access (Users and Access, "
            f"Integrations, App Store Connect API), or make a new key with that access. "
            f"Apple said: {error.detail}"
        )
    return error


# --- Signing material -------------------------------------------------------


def certificate_serial(cert_path: Path) -> str:
    """The serial number of a PEM certificate, as uppercase hex without leading zeros."""
    result = subprocess.run(
        ["openssl", "x509", "-in", str(cert_path), "-noout", "-serial"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0 or "=" not in result.stdout:
        raise SigningError(
            "openssl could not read the stored distribution certificate. Check that "
            "IOS_DISTRIBUTION_P12_BASE64 holds the base64 of a .p12 with the certificate in it."
        )
    return result.stdout.strip().split("=", 1)[1].upper().lstrip("0") or "0"


STORED_CERT_HELP = (
    "Make a new Apple Distribution certificate from the same private key and store it "
    "again as IOS_DISTRIBUTION_P12_BASE64 and IOS_DISTRIBUTION_P12_PASSWORD "
    "(docs/ios-testflight.md)."
)


def find_certificate(token: str, serial: str) -> dict:
    """The team's certificate with this serial number; refuses a missing or expired one."""
    query = urllib.parse.urlencode({"filter[serialNumber]": serial, "limit": 200})
    for cert in api_request(token, "GET", f"/v1/certificates?{query}").get("data", []):
        attrs = cert.get("attributes", {})
        if str(attrs.get("serialNumber", "")).upper().lstrip("0") != serial:
            continue
        if attrs.get("certificateType") not in CERTIFICATE_TYPES:
            raise SigningError(
                f"The stored certificate (serial {serial}) is a {attrs.get('certificateType')} "
                f"certificate, not a distribution one. {STORED_CERT_HELP}"
            )
        expires = str(attrs.get("expirationDate", ""))
        if expires and expires[:19] <= time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()):
            raise SigningError(
                f"The stored distribution certificate expired on {expires[:10]}. {STORED_CERT_HELP}"
            )
        return cert
    raise SigningError(
        f"The stored distribution certificate (serial {serial}) is not on the team any more; "
        f"it was revoked or has expired. {STORED_CERT_HELP}"
    )


def find_bundle_id(token: str, identifier: str) -> str | None:
    query = urllib.parse.urlencode({"filter[identifier]": identifier, "limit": 200})
    for item in api_request(token, "GET", f"/v1/bundleIds?{query}").get("data", []):
        if item.get("attributes", {}).get("identifier") == identifier:
            return item["id"]
    return None


def ensure_bundle_id(token: str, identifier: str) -> tuple[str, bool]:
    existing = find_bundle_id(token, identifier)
    if existing:
        return existing, False
    body = {
        "data": {
            "type": "bundleIds",
            "attributes": {"identifier": identifier, "name": APP_NAME, "platform": "IOS"},
        }
    }
    return api_request(token, "POST", "/v1/bundleIds", body)["data"]["id"], True


def find_profile(token: str, bundle_id: str, certificate_id: str) -> dict | None:
    """An active App Store profile for this bundle id that names this certificate."""
    query = urllib.parse.urlencode(
        {
            "filter[profileType]": PROFILE_TYPE,
            "filter[profileState]": "ACTIVE",
            "include": "bundleId,certificates",
            "limit": 200,
            "limit[certificates]": 50,
        }
    )
    reply = api_request(token, "GET", f"/v1/profiles?{query}")
    best = None
    for profile in reply.get("data", []):
        rel = profile.get("relationships", {})
        bundle = (rel.get("bundleId", {}).get("data") or {}).get("id")
        certs = [c.get("id") for c in rel.get("certificates", {}).get("data") or []]
        if bundle != bundle_id or certificate_id not in certs:
            continue
        if not profile.get("attributes", {}).get("profileContent"):
            continue
        expires = str(profile["attributes"].get("expirationDate", ""))
        if best is None or expires > str(best["attributes"].get("expirationDate", "")):
            best = profile
    return best


def create_profile(token: str, name: str, bundle_id: str, certificate_id: str) -> dict:
    body = {
        "data": {
            "type": "profiles",
            "attributes": {"name": name, "profileType": PROFILE_TYPE},
            "relationships": {
                "bundleId": {"data": {"type": "bundleIds", "id": bundle_id}},
                "certificates": {"data": [{"type": "certificates", "id": certificate_id}]},
            },
        }
    }
    return api_request(token, "POST", "/v1/profiles", body)["data"]


def find_app_id(token: str, identifier: str) -> str | None:
    query = urllib.parse.urlencode({"filter[bundleId]": identifier})
    for app in api_request(token, "GET", f"/v1/apps?{query}").get("data", []):
        if app.get("attributes", {}).get("bundleId", identifier) == identifier:
            return app["id"]
    return None


def app_record_exists(token: str, identifier: str) -> bool:
    return find_app_id(token, identifier) is not None


def _version_key(text: str) -> tuple[int, ...] | None:
    try:
        return tuple(int(part) for part in text.split("."))
    except ValueError:
        return None


def uploaded_builds(token: str, app_id: str) -> list[tuple[str, str]]:
    """(short version, build number) of every build App Store Connect lists."""
    query = urllib.parse.urlencode(
        {
            "filter[app]": app_id,
            "include": "preReleaseVersion",
            "fields[builds]": "version,preReleaseVersion",
            "fields[preReleaseVersions]": "version",
            "sort": "-uploadedDate",
            "limit": "200",
        }
    )
    reply = api_request(token, "GET", f"/v1/builds?{query}")
    short = {
        item["id"]: item.get("attributes", {}).get("version", "")
        for item in reply.get("included", [])
        if item.get("type") == "preReleaseVersions"
    }
    found = []
    for build in reply.get("data", []):
        related = build.get("relationships", {}).get("preReleaseVersion", {}).get("data") or {}
        found.append(
            (short.get(related.get("id"), ""), build.get("attributes", {}).get("version", ""))
        )
    return found


def next_build_number(
    builds: list[tuple[str, str]], short_version: str, run_number: int
) -> tuple[int, int]:
    """(next number, highest number seen). Counts only plain integer build numbers
    and refuses a short version lower than one already uploaded."""
    wanted = _version_key(short_version)
    highest = 0
    for short, number in builds:
        seen = _version_key(short)
        if wanted is not None and seen is not None and seen > wanted:
            raise SigningError(
                f"TestFlight already has version {short}, which is higher than this build's "
                f"{short_version}. Apple refuses an upload to a closed version. Raise the "
                "version in pyproject.toml first (see docs/ios-testflight.md)."
            )
        if number.isdigit():
            highest = max(highest, int(number))
    return max(highest, run_number) + 1, highest


# --- External testers ---------------------------------------------------------


def find_build(
    token: str, app_id: str, number: str, attempts: int = 30, pause: float = 20.0
) -> dict:
    """The uploaded build with this build number, once Apple has processed it."""
    query = urllib.parse.urlencode(
        {"filter[app]": app_id, "filter[version]": number, "limit": "10"}
    )
    state = "not listed yet"
    for attempt in range(attempts):
        builds = api_request(token, "GET", f"/v1/builds?{query}").get("data", [])
        if builds:
            state = builds[0].get("attributes", {}).get("processingState", "")
            if state == "VALID":
                return builds[0]
            if state in ("FAILED", "INVALID"):
                break
        if attempt + 1 < attempts:
            time.sleep(pause)
    raise SigningError(
        f"Build {number} is {state} in App Store Connect, so it was not sent to external "
        "testers. Internal testers still get it once Apple finishes processing."
    )


def find_external_group(token: str, app_id: str, name: str) -> dict:
    query = urllib.parse.urlencode({"limit": "200"})
    for group in api_request(token, "GET", f"/v1/apps/{app_id}/betaGroups?{query}").get("data", []):
        attributes = group.get("attributes", {})
        if attributes.get("name") == name and not attributes.get("isInternalGroup"):
            return group
    raise SigningError(
        f"There is no external TestFlight group named {name!r} for this app (App Store "
        "Connect, the app, TestFlight, External Testing). The build was uploaded for "
        "internal testers only."
    )


def review_submitted(token: str, build_id: str) -> bool:
    try:
        reply = api_request(token, "GET", f"/v1/builds/{build_id}/betaAppReviewSubmission")
    except ApiError as error:
        if error.status == 404:
            return False
        raise
    return bool(reply.get("data"))


def distribute(token: str, bundle_id: str, number: str, group_name: str) -> str:
    """Add build ``number`` to the external group and submit it; returns a summary."""
    app_id = find_app_id(token, bundle_id)
    if app_id is None:
        raise SigningError(MISSING_APP_HELP.format(bundle=bundle_id))
    group = find_external_group(token, app_id, group_name)
    build = find_build(token, app_id, number)
    api_request(
        token,
        "POST",
        f"/v1/betaGroups/{group['id']}/relationships/builds",
        {"data": [{"type": "builds", "id": build["id"]}]},
    )
    if review_submitted(token, build["id"]):
        return f"Build {number} added to {group_name}; it was already submitted for beta review."
    api_request(
        token,
        "POST",
        "/v1/betaAppReviewSubmissions",
        {
            "data": {
                "type": "betaAppReviewSubmissions",
                "relationships": {"build": {"data": {"type": "builds", "id": build["id"]}}},
            }
        },
    )
    return (
        f"Build {number} added to {group_name} and submitted for beta review. Its testers "
        "get it once Apple approves it."
    )


MISSING_APP_HELP = (
    "There is no App Store Connect app record for bundle id {bundle}, the id "
    "tools/build_ios.py stamps into the app. The bundle id in App Store Connect must match "
    "it exactly (App Store Connect, the app, App Information). Nothing was built."
)


# --- Commands ---------------------------------------------------------------


def _credentials() -> tuple[str, str, Path]:
    try:
        key_id = os.environ["ASC_KEY_ID"]
        issuer = os.environ["ASC_ISSUER_ID"]
        key_path = Path(os.environ["ASC_KEY_PATH"])
    except KeyError as missing:
        raise SigningError(
            f"{missing.args[0]} is not set; the App Store Connect key is needed."
        ) from None
    if not key_id or not issuer:
        raise SigningError("ASC_KEY_ID or ASC_ISSUER_ID is empty; set the repository secrets.")
    if not key_path.is_file() or key_path.stat().st_size == 0:
        raise SigningError(
            "The App Store Connect key file is missing or empty (ASC_KEY_P8_BASE64)."
        )
    return key_id, issuer, key_path


def _token() -> str:
    key_id, issuer, key_path = _credentials()
    return make_token(key_id, issuer, key_path)


def cmd_prepare(args: argparse.Namespace) -> int:
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    cert_path = Path(args.cert) if args.cert else out / CERT_FILE
    if not cert_path.is_file() or cert_path.stat().st_size == 0:
        raise SigningError(
            f"The stored distribution certificate was not exported to {cert_path}. "
            "Check IOS_DISTRIBUTION_P12_BASE64 and IOS_DISTRIBUTION_P12_PASSWORD."
        )
    serial = certificate_serial(cert_path)
    token = _token()
    try:
        cert = find_certificate(token, serial)
        print(
            f"Key accepted. Signing with the stored distribution certificate (id {cert['id']}, "
            f"expires {str(cert['attributes'].get('expirationDate', ''))[:10]})."
        )
        bundle, created = ensure_bundle_id(token, args.bundle_id)
        print(
            f"Bundle id {args.bundle_id}: {'registered now' if created else 'already registered'}."
        )
        profile = find_profile(token, bundle, cert["id"])
        if profile is not None:
            name = profile["attributes"].get("name")
            print(f"Reusing the App Store profile {name!r} (id {profile['id']}).")
        else:
            profile = create_profile(
                token, f"Freight Fate App Store {cert['id']}", bundle, cert["id"]
            )
            print(
                f"Made an App Store profile for the stored certificate (id {profile['id']}). "
                "Later runs reuse it."
            )
        profile_path = out / PROFILE_FILE
        profile_path.write_bytes(base64.b64decode(profile["attributes"]["profileContent"]))
    except ApiError as error:
        raise explain(error) from None
    return 0


def cmd_check_app(args: argparse.Namespace) -> int:
    token = _token()
    try:
        found = app_record_exists(token, args.bundle_id)
    except ApiError as error:
        raise explain(error) from None
    if not found:
        raise SigningError(MISSING_APP_HELP.format(bundle=args.bundle_id))
    print(f"App record for {args.bundle_id} exists.")
    return 0


def cmd_next_build(args: argparse.Namespace) -> int:
    token = _token()
    short_version = _BUILD_IOS.store_version(_BUILD_IOS.load_build_release().project_version())
    try:
        app_id = find_app_id(token, args.bundle_id)
        if app_id is None:
            raise SigningError(MISSING_APP_HELP.format(bundle=args.bundle_id))
        builds = uploaded_builds(token, app_id)
    except ApiError as error:
        raise explain(error) from None
    number, highest = next_build_number(builds, short_version, args.run_number)
    print(
        f"Version {short_version}: {len(builds)} earlier builds, highest build number "
        f"{highest}; this build will be {number}.",
        file=sys.stderr,
    )
    print(number)
    return 0


def cmd_distribute(args: argparse.Namespace) -> int:
    if not args.build_number:
        raise SigningError("distribute needs --build-number.")
    token = _token()
    try:
        print(distribute(token, args.bundle_id, args.build_number, args.group))
    except ApiError as error:
        raise explain(error) from None
    return 0


def cmd_cleanup(args: argparse.Namespace) -> int:
    out = Path(args.out)
    for name in (CERT_FILE, PROFILE_FILE, P12_FILE):
        (out / name).unlink(missing_ok=True)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    for name, func in (
        ("prepare", cmd_prepare),
        ("check-app", cmd_check_app),
        ("next-build", cmd_next_build),
        ("distribute", cmd_distribute),
        ("cleanup", cmd_cleanup),
    ):
        p = sub.add_parser(name)
        p.add_argument("--bundle-id", default=BUNDLE_ID)
        p.add_argument("--out", default="build/ios-signing")
        p.add_argument("--label", default="manual", help="run label for names (no secrets)")
        p.add_argument("--run-number", type=int, default=0, help="lowest build number to allow")
        p.add_argument("--cert", default="", help="PEM of the stored certificate (prepare)")
        p.add_argument("--build-number", default="", help="uploaded build (distribute)")
        p.add_argument("--group", default="Friends", help="external tester group (distribute)")
        p.set_defaults(func=func)
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except SigningError as error:
        print(f"::error::{error}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
