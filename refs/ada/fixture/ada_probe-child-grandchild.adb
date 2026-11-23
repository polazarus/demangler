package body Ada_Probe.Child.Grandchild is
   procedure Deep is
   begin
      null;
   end Deep;

   function Deep_Value return Integer is
   begin
      return 2;
   end Deep_Value;
end Ada_Probe.Child.Grandchild;
