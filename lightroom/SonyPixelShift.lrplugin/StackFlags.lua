-- Folder-stack flags use catalog metadata only; no RAW helper or UI automation.
local LrApplication = import 'LrApplication'
local LrDialogs = import 'LrDialogs'
local LrTasks = import 'LrTasks'
local M = {}
local busy = false
local actions = {
    [1] = { title = 'Flag entire stack', verb = 'Flagged' },
    [-1] = { title = 'Reject entire stack', verb = 'Rejected' },
    [0] = { title = 'Unflag entire stack', verb = 'Unflagged' },
}
local function check(condition, message)
    if not condition then error(message, 0) end
end
local function identifier(photo)
    local id = photo.localIdentifier
    check(type(id) == 'number', 'Could not identify a selected photo. Retry the command.')
    return id
end
local function collect(selected)
    local photos, seen, visited = {}, {}, {}
    local function add(photo)
        local id = identifier(photo)
        if not seen[id] then seen[id] = true; photos[#photos+1] = photo end
    end
    for _, photo in ipairs(selected) do
        local id = identifier(photo)
        if not visited[id] then
            if photo:getRawMetadata('isInStackInFolder') then
                local members = photo:getRawMetadata('stackInFolderMembers')
                local count = photo:getRawMetadata('countStackInFolderMembers')
                check(type(members) == 'table' and type(count) == 'number' and
                    count > 0 and #members == count,
                    'Could not read the complete folder stack. Retry the command.')
                local ids, unique = {}, 0
                for _, member in ipairs(members) do
                    local memberId = identifier(member)
                    if not ids[memberId] then ids[memberId] = true; unique = unique+1 end
                end
                check(unique == count and ids[id],
                    'Folder stack membership changed. Retry the command.')
                for _, member in ipairs(members) do
                    visited[identifier(member)] = true
                    add(member)
                end
            else
                visited[id] = true
                add(photo)
            end
        end
    end
    return photos
end
function M.apply(catalog, status)
    local action = actions[status]
    check(action, 'Invalid stack flag operation.')
    -- Collection stacks can differ from folder stacks. Do not expand them using
    -- folder metadata while implying they represent the visible collection stack.
    local sources = catalog:getActiveSources()
    check(type(sources) == 'table' and #sources > 0,
        'Open a folder in the Library Folders panel, then retry.')
    for _, source in ipairs(sources) do
        check(type(source) ~= 'string' and source:type() == 'LrFolder',
            'Open a folder in the Library Folders panel, then retry. Collection stacks are not supported.')
    end
    -- getTargetPhotos() falls back to the entire filmstrip when nothing is
    -- selected. Never let an empty selection flag a whole folder accidentally.
    check(catalog:getTargetPhoto(), 'Select a photo or stack first.')
    local selected = catalog:getTargetPhotos()
    check(#selected > 0, 'Select a photo or stack first.')
    -- Read membership before the write gate. Include hidden/collapsed members,
    -- deduplicate selected siblings and handle selected unstacked photos too.
    local photos = collect(selected)
    local callbackRan = false
    local result = catalog:withWriteAccessDo(action.title, function()
        for _, photo in ipairs(photos) do photo:setRawMetadata('pickStatus', status) end
        callbackRan = true
    end, { timeout = 30 })
    check(callbackRan and result == 'executed', 'Catalog busy. Retry the stack flag command.')
    return #photos
end
function M.start(status)
    LrTasks.startAsyncTask(function()
        if busy then LrDialogs.showBezel('Stack flags are updating. Try again.', 1.5); return end
        busy = true
        local count
        local ok, problem = LrTasks.pcall(function()
            count = M.apply(LrApplication.activeCatalog(), status)
        end)
        busy = false
        if ok then
            LrDialogs.showBezel(actions[status].verb .. ' ' .. tostring(count) ..
                (count == 1 and ' photo' or ' photos'), 1.5)
        else
            LrDialogs.message('Stack flags', tostring(problem), 'critical')
        end
    end)
end
return M
