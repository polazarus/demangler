/* PE/COFF decoration probe: imported stdcall C++. */
__declspec(dllimport) int __stdcall imported_cpp_stdcall(int a, int b);
int call_imported_cpp_stdcall(int a) { return imported_cpp_stdcall(a, 2); }
