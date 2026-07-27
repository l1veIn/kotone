# Privacy

Kotone is designed as a local-first voice input tool.

- Microphone audio is processed by the speech recognition model on the user's computer.
- Kotone does not include analytics, advertising SDKs, account login, or telemetry.
- Network access is used to download speech models from the sources listed in the application. Users can choose the official source, a mirror, and an optional GitHub proxy.
- Settings, game profiles, recognition history, optional evaluation recordings, and downloaded models are stored under `~/.kotone`.
- Recognition history can be disabled or cleared in the application. Evaluation recording and history audio are disabled by default.
- Uninstalling the application preserves `~/.kotone` so that large models are not lost accidentally. Users can delete that directory manually when they also want to remove all local data.

Kotone injects recognized text into the currently selected target window. It does not read game memory, alter game files, or communicate with game servers.
