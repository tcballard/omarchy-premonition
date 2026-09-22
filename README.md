<h1 align="center">Omarchy Premonition</h1>

<p align="center">
  <a href="https://github.com/tcballard/omarchy-badges"><img src="https://raw.githubusercontent.com/tcballard/omarchy-badges/75975e5b5bf75e7ede3764bcd2950046f7abfe2c/badges/v1/omarchy-plugin.svg" alt="Built for Omarchy: Plugin" height="24"></a>
</p>

**Turn an error into a patch you can review.**

Send an error from a repository you choose to a bounded Codex investigation. Premonition presents a candidate patch for inspection and changes files only after you explicitly choose **Apply**.

## Everyday use

Select an allowlisted repository and submit the error from selected text, the clipboard or stdin. Inspect the proposed diff, then apply or dismiss it. Premonition checks the patch against the repository again before applying.

## Install

Compatible Omarchy Quattro, a systemd user session, Git, Rust 1.98.0 for the source build, and a configured Codex CLI.

Follow the [source installation guide](GUIDE.md#source-install-for-review) to build the runtime, select an initial repository and install the user service. Then review and enable **Premonition** in **Setup → Plugins**. This plugin requires its native service; clone-only installation is not enough.

## Update and remove

This plugin also has a native runtime. Follow the [runtime and plugin instructions](GUIDE.md#removal) when updating or removing it. The shell plugin can be removed with `omarchy plugin remove io.github.tcballard.premonition`.

## A few useful details

**v0.1.0 source installation.** Machine-independent acceptance is complete; hands-on desktop acceptance remains separate. [Validation](docs/validation.md) · [Pinned compatibility](docs/pins.md)

A patch that applies is still a candidate for your review. Premonition does not run tests, create commits or open PRs. [Workflow and limits →](GUIDE.md#safety-boundary)

[Usage and development guide](GUIDE.md) · [Report a bug](https://github.com/tcballard/omarchy-premonition/issues)

[Licence licensed](LICENSE).

<!-- Preserve links to sections now in the guide. -->
<a id="dependencies"></a>
<a id="evidence"></a>
<a id="licence"></a>
<a id="removal"></a>
<a id="safety-boundary"></a>
<a id="source-install-for-review"></a>
<a id="stable-command-surface"></a>
<a id="status"></a>
<a id="what-the-workflow-does"></a>

[Looking for the previous detailed sections? Open the full guide →](GUIDE.md)
