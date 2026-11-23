! External (non-module) procedures and data. gfortran appends one underscore
! to each external name by default (`-funderscoring`).
subroutine ext_sub()
end subroutine ext_sub

subroutine my_ext_sub(a)
  integer, intent(in) :: a
end subroutine my_ext_sub

subroutine ext_underscored_(b)
  integer, intent(in) :: b
end subroutine ext_underscored_

subroutine ext_with_digits_1()
end subroutine ext_with_digits_1

real function ext_fun(x) result(y)
  real, intent(in) :: x
  y = x
end function ext_fun

subroutine ext_c(x) bind(c)
  integer, intent(in) :: x
end subroutine ext_c

block data probe_block
  common /probe_blk/ shared_value
  real :: shared_value
end block data probe_block
