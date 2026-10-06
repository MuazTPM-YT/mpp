# Muaz++ for VS Code

Highlighting, errors as you type, hover docs, completion, go to definition,
outline and formatting for `.mpp` files, plus Run / Test commands.

Needs the `mpp` binary on your PATH (or set `mpp.path` in settings).

Build and install:

```sh
cd editors/vscode
npm install
npm run package                     # makes muazpp-0.1.0.vsix
code --install-extension muazpp-0.1.0.vsix
```
