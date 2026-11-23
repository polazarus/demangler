! Main program: gfortran names it `MAIN__` and emits the C `main` wrapper.
program probe_main
  use fprobe
  use fprobe_child
  use fprobe_generic
  implicit none
  real, external :: ext_fun, double_it
  type(point) :: p

  p = point(1.0, 2.0)
  call hello()
  print *, add(1, 2), p%norm(), answer, total(1, 2), twice(scale_factor)
  call outer(3)
  call with_entry(4)
  call with_entry2(5)
  call trailing_()
  call c_proc(6)
  call do_child()
  print *, child_value(7)
  print *, apply(1.0), apply(2)
  call set_op(double_it)
  call ext_sub()
  call my_ext_sub(1)
  call ext_underscored_(2)
  call ext_with_digits_1()
  print *, ext_fun(1.0)
  call ext_c(8)
end program probe_main
