# Scrobble Scrubber app

Scrobble Scrubber is the native Dioxus interface for reviewing and applying Last.fm
metadata cleanup. The same Rust backend runs in the Linux desktop and Android builds.

## Mobile login

On a fresh install, enter a Last.fm username and password in the app's sign-in form.
Authentication is sent directly to Last.fm over HTTPS. The password is discarded after
login; the resulting session cookie and the optional API key are stored in the app's
private sandbox so later launches can reuse them.

The API key is optional for editing. It enables recent-scrobble synchronization when
provided.

## Local builds

```sh
just app-desktop-release
just app-android-build
```

An Android release build also needs these environment variables:

```text
ANDROID_SIGNING_KEYSTORE_BASE64
ANDROID_SIGNING_KEYSTORE_PASSWORD
ANDROID_SIGNING_KEY_ALIAS
ANDROID_SIGNING_KEY_PASSWORD
```

Then run `just app-android-release`.

## Publishing a version

Keep the versions in the `scrobble-store`, `scrobble-scrubber`, and
`scrobble-scrubber-app` manifests synchronized, regenerate `Cargo.lock`, and push an
annotated tag named `scrobble-scrubber-vX.Y.Z`. The release workflow validates the tag,
builds a signed Android APK and AAB plus a Linux desktop package, and publishes the
GitHub release only after all artifacts succeed.
