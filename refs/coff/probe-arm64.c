/* PE/COFF decoration probe: ARM64 has no calling-convention decoration.
 *
 * `__cdecl`, `__stdcall` and `__fastcall` all produce the bare name, and an
 * import pointer is the only decoration (`__imp_`).
 */
__declspec(dllexport) int arm_plain(int a, int b) { return a + b; }
__declspec(dllexport) int __stdcall arm_stdcall(int a, int b) { return a + b; }
__declspec(dllexport) int __fastcall arm_fastcall(int a, int b) { return a - b; }

__declspec(dllimport) int imported_arm(int a, int b);

int call_arm(int a) { return imported_arm(a, 1); }
