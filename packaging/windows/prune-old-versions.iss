// Included into [Code] of agentmux.iss.
//
// Each version's setup puts agentmux-<ver>.exe, agentmux-<ver>.dll and agentmux-srv-<ver>-windows.<arch>.exe into
// runtime\ beside the others (a running old version's files are never overwritten), and nothing took the old ones
// away: about 95 MB a version. The launcher prunes them itself, but in an all-users install under Program Files it
// runs unelevated and every removal is refused. Setup runs elevated there, so it does the same work after its own
// files are in place: it keeps the version being installed and the newest older one (the way back), and removes the
// files of every other older version. The rule is the launcher's (crates/launcher/src/old_versions.rs): only those
// three names, only versions older than the one being installed, compared as numbers (0.60.9 is older than 0.60.10).
// A file still in use cannot be deleted while it runs; it is skipped and a later setup tries again.

{ major.minor.patch, digits only, at most six digits each, as one number; -1 for anything else. }
function VersionKey(const S: String): Int64;
var
  Parts: array[0..2] of String;
  Nums: array[0..2] of Int64;
  Rest: String;
  I, P, N: Integer;
begin
  Result := -1;
  Rest := S;
  for I := 0 to 2 do
  begin
    if I < 2 then
    begin
      P := Pos('.', Rest);
      if P = 0 then
        Exit;
      Parts[I] := Copy(Rest, 1, P - 1);
      Rest := Copy(Rest, P + 1, Length(Rest));
    end
    else
      Parts[I] := Rest;
    if (Length(Parts[I]) = 0) or (Length(Parts[I]) > 6) then
      Exit;
    for N := 1 to Length(Parts[I]) do
      if (Parts[I][N] < '0') or (Parts[I][N] > '9') then
        Exit;
    Nums[I] := StrToInt(Parts[I]);
  end;
  Result := (Nums[0] * 1000000 + Nums[1]) * 1000000 + Nums[2];
end;

{ The version in one of the three versioned file names; -1 for every other name. }
function VersionOfRuntimeName(const Name: String): Int64;
var
  Rest, Tail: String;
  P: Integer;
begin
  Result := -1;
  if Copy(Name, 1, 13) = 'agentmux-srv-' then
  begin
    { agentmux-srv-<ver>-windows.<arch>.exe }
    Rest := Copy(Name, 14, Length(Name));
    P := Pos('-windows.', Rest);
    if (P > 0) and (Length(Rest) > 4) and (Copy(Rest, Length(Rest) - 3, 4) = '.exe') then
      Result := VersionKey(Copy(Rest, 1, P - 1));
  end
  else if Copy(Name, 1, 9) = 'agentmux-' then
  begin
    { agentmux-<ver>.exe and agentmux-<ver>.dll }
    Rest := Copy(Name, 10, Length(Name));
    if Length(Rest) > 4 then
    begin
      Tail := Copy(Rest, Length(Rest) - 3, 4);
      if (Tail = '.exe') or (Tail = '.dll') then
        Result := VersionKey(Copy(Rest, 1, Length(Rest) - 4));
    end;
  end;
end;

{ Removes the files of the versions older than Running from RuntimeDir, except those of the newest older one.
  Returns how many files it removed. }
function PruneOldVersions(const RuntimeDir, Running: String): Integer;
var
  Find: TFindRec;
  RunningKey, Key, Keep: Int64;
begin
  Result := 0;
  RunningKey := VersionKey(Running);
  if RunningKey < 0 then
    Exit;
  Keep := -1;
  if FindFirst(RuntimeDir + '\agentmux-*', Find) then
  begin
    try
      repeat
        if (Find.Attributes and FILE_ATTRIBUTE_DIRECTORY) = 0 then
        begin
          Key := VersionOfRuntimeName(Find.Name);
          if (Key >= 0) and (Key < RunningKey) and (Key > Keep) then
            Keep := Key;
        end;
      until not FindNext(Find);
    finally
      FindClose(Find);
    end;
  end;
  if FindFirst(RuntimeDir + '\agentmux-*', Find) then
  begin
    try
      repeat
        if (Find.Attributes and FILE_ATTRIBUTE_DIRECTORY) = 0 then
        begin
          Key := VersionOfRuntimeName(Find.Name);
          if (Key >= 0) and (Key < RunningKey) and (Key <> Keep) then
          begin
            if DeleteFile(RuntimeDir + '\' + Find.Name) then
              Result := Result + 1
            else
              Log('old versions: could not remove ' + Find.Name + ' (in use?)');
          end;
        end;
      until not FindNext(Find);
    finally
      FindClose(Find);
    end;
  end;
  Log('old versions: removed ' + IntToStr(Result) + ' files from ' + RuntimeDir);
end;
