$null = [Console]::In.ReadToEnd()
[Console]::Out.WriteLine('{"decision":"deny","reason":"HARNESSMIX_HOOK_DENIED"}')
