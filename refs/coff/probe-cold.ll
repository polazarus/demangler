; Hot/cold-split probe for the `.cold.N` fragment suffix.
;
; LLVM's HotColdSplitting pass outlines a cold block into a separate function
; named `<name>.cold.N`, and the COFF backend appends that suffix *after* the
; platform decoration. The names here are valid on every COFF triple:
;
;   opt-19 -passes=hotcoldsplit -S probe-cold.ll -o - \
;     | llc-19 -mtriple=<triple> -filetype=obj -o <object>
;
; The pass needs branch weights and a function entry count, hence the `!prof`
; metadata; without profile data it never outlines a region.
declare void @abort_now() cold noreturn

define dso_local i32 @"?cold_cpp@@YAHH@Z"(i32 %a, i32 %b) !prof !1 {
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

define dso_local i32 @"cold_plain"(i32 %a, i32 %b) !prof !1 {
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

define dso_local i32 @"_Z11cpp_stdcallii"(i32 %a, i32 %b) !prof !1 {
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
