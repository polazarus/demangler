/* PE/COFF decoration probe: C calling conventions and dllimport. */
__declspec(dllexport) int __stdcall coff_stdcall(int a, int b) { return a + b; }
__declspec(dllexport) int __fastcall coff_fastcall(int a, int b) { return a - b; }
__declspec(dllexport) int __cdecl coff_cdecl(int a, int b) { return a * b; }

__declspec(dllimport) int __stdcall imported_stdcall(int a, int b);
__declspec(dllimport) int __cdecl imported_cdecl(int a, int b);
__declspec(dllimport) int __fastcall imported_fastcall(int a, int b);

int call_imports(int a) {
    return imported_stdcall(a, 1) + imported_cdecl(a, 2) + imported_fastcall(a, 3);
}
