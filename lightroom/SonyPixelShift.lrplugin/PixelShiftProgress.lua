-- Native Lightroom scope and incremental engine-log progress. Keep this module
-- project-named and use plain tables; no metatable/global loader dependency.
local M = {}
function M.new(scope, count, tasks)
    local self = {scope=scope, count=count, tasks=tasks, offset=0, tail=''}
    function self:stage(caption)
        self.scope:setIndeterminate()
        self.scope:setCaption(caption)
    end
    function self:line(line)
        local region,total=line:match('^output region (%d+)/(%d+)$')
        region,total=tonumber(region),tonumber(total)
        if region and total and total>0 and total<=1000000 and region>=1 and region<=total then
            self.scope:setPortionComplete(region,total+1) -- Reserve completion for catalog import.
            self.scope:setCaption('Combining and saving ' .. region .. ' of ' .. total)
            return
        end
        local index, count = line:match('^decode (%d+)/(%d+): ')
        index, count = tonumber(index), tonumber(count)
        if index and count == self.count and index >= 1 and index <= count then
            -- The logged frame has just started; only preceding frames are done.
            self.scope:setPortionComplete(index-1, count)
            self.scope:setCaption('Decoding RAW ' .. index .. ' of ' .. count)
        elseif line == 'merge and motion correction' then
            self:stage('Aligning and merging')
        elseif line:match('^writing %d+x%d+ DNG$') then
            self:stage('Writing DNG')
        elseif line:match('^writing %d+x%d+ TIF') then
            self:stage('Writing TIFF')
        elseif line:match('^complete in ') then
            self:stage('Checking result')
        end
    end
    function self:poll(path)
        local file = io.open(path, 'rb')
        if not file then return end
        local position = file:seek('set', self.offset)
        local chunk = position and file:read(16384) or nil
        file:close()
        if not chunk or #chunk == 0 then return end
        self.offset = self.offset + #chunk
        local text = self.tail .. chunk
        local consumed = 1
        for line, nextPosition in text:gmatch('([^\n]*)\n()') do
            self:line(line:gsub('\r$', ''))
            consumed = nextPosition
        end
        -- Bound the parser even for malformed logs or very long filenames.
        self.tail = text:sub(consumed):sub(-8192)
    end
    function self:run(command, logPath)
        self:stage('Checking capture')
        local finished, status, problem = false, nil, nil
        self.tasks.startAsyncTask(function()
            local ok, result = self.tasks.pcall(function() return self.tasks.execute(command) end)
            if ok then status = result else problem = result end
            finished = true
        end)
        while not finished do
            self:poll(logPath)
            self.tasks.sleep(0.15)
        end
        self:poll(logPath)
        if problem then error(problem, 0) end
        return status
    end
    function self:done()
        if self.closed then return end
        self.closed = true
        if not self.scope:isDone() then self.scope:done() end
    end
    -- Whole-frame decode/output does not yet support safe interruption.
    scope:setCancelable(false)
    scope:setPausable(false, false)
    self:stage('Preparing')
    return self
end
return M
