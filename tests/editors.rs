// Vim/Neovim plugin in real headless Neovim (skipped when nvim is not installed)
use std::process::Command;

#[test]
fn neovim_plugin_and_lsp() {
    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("nvim not installed; skipping");
        return;
    }
    let root = env!("CARGO_MANIFEST_DIR");
    let dir = std::env::temp_dir().join(format!("mpp_nvim_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let demo = dir.join("demo.mpp");
    std::fs::write(&demo, "import stats\nfn add(a,b){\nreturn a+b\n}\nx = add(1, 2)\nprint(f\"x={x}\", undefined_name)\n").unwrap();
    let result = dir.join("result.txt");
    let bin_dir = std::path::Path::new(env!("CARGO_BIN_EXE_mpp")).parent().unwrap();
    let path = format!("{}:{}", bin_dir.display(), std::env::var("PATH").unwrap_or_default());
    let status = Command::new("nvim")
        .args(["--headless", "--clean", "--cmd", &format!("set rtp^={root}/editors/vim")])
        .args(["-c", &format!("let g:demo='{}'", demo.display()), "-c", &format!("let g:result='{}'", result.display())])
        .args(["-c", &format!("luafile {root}/tests/fixtures/nvim_check.lua")])
        .env("PATH", path)
        .status()
        .unwrap();
    assert!(status.success());
    let out = std::fs::read_to_string(&result).unwrap();
    assert_eq!(out, "ft=mpp\nsyn=mppKeyword,mppFunction,mppFString\nindent=4\ndiag=5\nfmt=fn add(a, b) {\n");
    std::fs::remove_dir_all(&dir).ok();
}
