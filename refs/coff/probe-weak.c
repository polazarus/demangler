/* PE/COFF decoration probe: COMDAT weak-alias section symbols.
 *
 * clang/LLVM wraps a weak definition in a COMDAT section whose symbol is
 * `.weak.<decorated name>.default`. The wrapper composes with the x86
 * `cdecl`/`stdcall`/`fastcall` decorations and with the undecorated x64 and
 * ARM64 spellings.
 */
__declspec(dllexport) __attribute__((weak)) int weak_cdecl(int a) { return a; }
__declspec(dllexport) __attribute__((weak)) int __stdcall weak_stdcall(int a, int b) {
    return a + b;
}
__declspec(dllexport) __attribute__((weak)) int __fastcall weak_fastcall(int a, int b) {
    return a - b;
}
