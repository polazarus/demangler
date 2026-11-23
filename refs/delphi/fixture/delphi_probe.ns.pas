unit delphi_probe.ns;

{ FPC namespaced-unit fixture: a dotted unit name keeps its dots in the linker
  name. Compiled with: fpc -O- -gw delphi_probe.ns.pas }

{$mode objfpc}{$H+}

interface

type
  TNamespaced = class
    procedure Method(a: LongInt);
  end;

procedure NamespacedProc(const s: AnsiString);

implementation

procedure TNamespaced.Method(a: LongInt);
begin
end;

procedure NamespacedProc(const s: AnsiString);
begin
end;

end.
