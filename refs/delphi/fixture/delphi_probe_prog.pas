program delphi_probe_prog;

{ FPC program fixture: program-scope symbols carry the `P$` unit marker.
  Compiled with: fpc -O- -gw delphi_probe_prog.pas }

{$mode objfpc}{$H+}

uses
  delphi_probe, delphi_probe.ns;

procedure ProgProc(a: LongInt);
begin
end;

begin
  ProgProc(1);
  UnitProc(2);
  NamespacedProc('x');
end.
