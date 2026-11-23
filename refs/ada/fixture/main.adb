with Ada_Probe;
with Ada_Probe.Child;
with Ada_Probe.Child.Grandchild;
procedure Main is
   X : Integer := 1;
   Y : Integer := 2;
begin
   Ada_Probe.Hello;
   Ada_Probe.Alpha;
   Ada_Probe.Beta;
   Ada_Probe.Gamma (1);
   Ada_Probe.Int_Gen.Do_It (1);
   Ada_Probe.Int_Swap (X, Y);
   Ada_Probe.Child.Do_Child;
   Ada_Probe.Child.Grandchild.Deep;
end Main;
