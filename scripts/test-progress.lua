-- Partial lines must not advance progress; diagnostic text is not a stage.
local Progress = require 'PixelShiftProgress'
local scope = {captions={}, closes=0}
function scope:setCancelable(value) assert(value == false) end
function scope:setPausable(pausable,cancelable) assert(pausable == false and cancelable == false) end
function scope:setIndeterminate() end
function scope:setCaption(value) self.captions[#self.captions+1] = value end
function scope:setPortionComplete(done,total) assert(done == 0 and total == 4) end
function scope:isDone() return self.closes > 0 end
function scope:done() self.closes = self.closes + 1 end
local progress = Progress.new(scope,4,{})
local path = os.tmpname()
local function append(value)
    local file=assert(io.open(path,'ab')); assert(file:write(value)); assert(file:close())
end
local ok,problem=pcall(function()
    progress:poll(path .. '.missing')
    append('decode 1/4: test.ARW')
    progress:poll(path)
    assert(scope.captions[#scope.captions] == 'Preparing')
    append('\r\nmerge and motion correction\nwriting 32x24 DNG\n')
    progress:poll(path)
    assert(scope.captions[2] == 'Decoding RAW 1 of 4')
    assert(scope.captions[3] == 'Aligning and merging')
    assert(scope.captions[4] == 'Writing DNG')
    append('decode 0/4: invalid\ndecode 1/16: wrong capture\nalignment residuals (sensor pixels): [(0,0)]\n')
    progress:poll(path)
    assert(#scope.captions == 4)
    append('complete in 2.0s; 0 pixels used reference fallback\n')
    progress:poll(path)
    assert(scope.captions[5] == 'Checking result' and scope.closes == 0)
    progress:done(); progress:done()
    assert(scope.closes == 1)
end)
os.remove(path)
assert(ok,problem)
print('Progress partial-log and lifecycle checks passed')
