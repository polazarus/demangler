/* PE/COFF decoration probe: C++ imports and stdcall mangling. */
__declspec(dllimport) int imported_cpp(int a);
__declspec(dllexport) int __stdcall cpp_stdcall(int a, int b) { return a + b; }
int call_imported_cpp(int a) { return imported_cpp(a); }
