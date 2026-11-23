/* PE/COFF decoration probe: `__vectorcall` spellings.
 *
 * `__vectorcall` is decorated `name@@N` on both x86 and x64. The target-aware
 * parser decodes it; the platform-agnostic parser does not, because `@@N` is
 * also a GNU symbol version on ELF.
 */
__declspec(dllexport) int __vectorcall coff_vectorcall(int a, int b) { return a + b; }
__declspec(dllexport) int __vectorcall coff_vectorcall_none(void) { return 0; }
__declspec(dllexport) int __vectorcall coff_vectorcall_one(int a) { return a; }
__declspec(dllexport) int __vectorcall coff_vectorcall_three(int a, int b, int c) {
    return a + b + c;
}

__declspec(dllimport) int __vectorcall imported_vectorcall(int a, int b);

int call_vectorcall(int a) { return imported_vectorcall(a, 1); }
