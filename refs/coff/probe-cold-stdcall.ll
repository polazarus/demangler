; Hot/cold-split probe for the x86 `stdcall` spelling `_name@N`.
;
; The decoration suffix precedes the fragment suffix in the object symbol:
; `_coff_stdcall@8.cold.1`. Only the 32-bit x86 target accepts the
; `x86_stdcallcc` calling convention, so this probe is separate from
; `probe-cold.ll`.
;
;   opt-19 -passes=hotcoldsplit -S probe-cold-stdcall.ll -o - \
;     | llc-19 -mtriple=i686-pc-windows-msvc -filetype=obj -o <object>
declare void @abort_now() cold noreturn

define dso_local x86_stdcallcc i32 @"\01_coff_stdcall@8"(i32 %a, i32 %b) !prof !1 {
entry:
  %cmp = icmp slt i32 %a, 0
  br i1 %cmp, label %cold, label %ok, !prof !0

cold:
  %c0 = mul i32 %a, 3
  %c1 = mul i32 %c0, 4
  %c2 = mul i32 %c1, 5
  %c3 = mul i32 %c2, 6
  %c4 = mul i32 %c3, 7
  %c5 = mul i32 %c4, 8
  call void @abort_now()
  unreachable

ok:
  %sum = add i32 %a, %b
  ret i32 %sum
}

!0 = !{!"branch_weights", i32 100000, i32 1}
!1 = !{!"function_entry_count", i64 100000}
