1. Create test.9z from a file of a few MB (right-click → 9-zip → Add to “test.9z”).

2. Make a damaged copy. Open PowerShell, paste this and press Enter. It copies test.9z on your Desktop to test-damaged.9z and scrambles 4 KB in the middle of the copy. The original stays untouched.

## Use following code for creating damaged .9z file

$d="$env:USERPROFILE\Desktop"; Copy-Item "$d\test.9z" "$d\test-damaged.9z"; $b=[IO.File]::ReadAllBytes("$d\test-damaged.9z"); $m=[int]($b.Length/2); for($i=$m;$i -lt [Math]::Min($m+4096,$b.Length);$i++){$b[$i]=$b[$i] -bxor 255}; [IO.File]::WriteAllBytes("$d\test-damaged.9z",$b)

## Use following code for creating damaged .zip file

$d="$env:USERPROFILE\Desktop"; Copy-Item "$d\test.zip" "$d\test-damaged.zip"; $b=[IO.File]::ReadAllBytes("$d\test-damaged.zip"); $m=[int]($b.Length/2); for($i=$m;$i -lt [Math]::Min($m+4096,$b.Length);$i++){$b[$i]=$b[$i] -bxor 255}; [IO.File]::WriteAllBytes("$d\test-damaged.zip",$b)

3. Open test-damaged.9z in 9-zip and click Test. It should report the damaged blocks and say that it can rebuild them.

4. Extract it. 9-zip repairs the damage on the way and tells you how many blocks it rebuilt. The extracted file opens normally.

For comparison, do steps 1 to 3 with a .zip: Test will report a data error, and that file cannot be recovered.