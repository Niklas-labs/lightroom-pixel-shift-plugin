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
    sync_rejection = { title = 'Sync rejection from stack top', verb = 'Synced rejection on' },
    reject_hidden = { title = 'Reject hidden photos in collapsed stacks', verb = 'Rejected' },
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
    local photos, stacks, seen, visited = {}, {}, {}, {}
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
                stacks[#stacks+1] = members
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
    return photos, stacks
end
local function rejectionChanges(stacks, rejectHidden)
    local changes, owned = {}, {}
    for _, members in ipairs(stacks) do
        local top, positions = nil, {}
        -- Member-array order is not specified by the SDK. Locate position 1,
        -- even when a lower photo is selected or the stack is collapsed.
        for _, member in ipairs(members) do
            local id = identifier(member)
            check(not owned[id], 'Folder stack membership changed. Retry the command.')
            owned[id] = true
            local position = member:getRawMetadata('stackPositionInFolder')
            check(type(position) == 'number' and position >= 1 and position <= #members and
                position == math.floor(position) and not positions[position],
                'Could not identify the complete folder stack order. Retry the command.')
            positions[position] = true
            if position == 1 then top = member end
        end
        check(top, 'Could not identify the folder stack top. Retry the command.')
        local topStatus = top:getRawMetadata('pickStatus')
        check(topStatus == -1 or topStatus == 0 or topStatus == 1,
            'Could not read the stack top rejection flag. Retry the command.')
        for _, member in ipairs(members) do
            if identifier(member) ~= identifier(top) then
                local current = member:getRawMetadata('pickStatus')
                check(current == -1 or current == 0 or current == 1,
                    'Could not read a stack member rejection flag. Retry the command.')
                local desired = current
                if rejectHidden or topStatus == -1 then desired = -1
                elseif current == -1 then desired = 0 end
                if desired ~= current then
                    changes[#changes+1] = { photo = member, status = desired }
                end
            end
        end
    end
    return changes
end
local function collapsedStacks(stacks)
    local result, positions = {}, {}
    for _, members in ipairs(stacks) do
        local collapsed = members[1]:getRawMetadata('stackInFolderIsCollapsed')
        check(type(collapsed) == 'boolean', 'Could not read whether a stack is collapsed. Retry the command.')
        for _, member in ipairs(members) do
            check(member:getRawMetadata('stackInFolderIsCollapsed') == collapsed,
                'Stack expansion changed. Retry the command.')
        end
        if collapsed then
            result[#result+1] = members
            for _, member in ipairs(members) do
                positions[identifier(member)] = member:getRawMetadata('stackPositionInFolder')
            end
        end
    end
    return result, positions
end
local function verifyCollapsed(stacks, positions)
    -- A busy catalog can yield while waiting for write access. Do not reject a
    -- stack the user has since expanded, or write to a newly promoted top.
    for _, members in ipairs(stacks) do
        local expected, seen = {}, {}
        for _, member in ipairs(members) do expected[identifier(member)] = true end
        local current = members[1]:getRawMetadata('stackInFolderMembers')
        check(type(current) == 'table' and #current == #members and
            members[1]:getRawMetadata('countStackInFolderMembers') == #members,
            'Stack membership changed. Retry the command.')
        for _, member in ipairs(current) do
            local id = identifier(member)
            check(expected[id] and not seen[id], 'Stack membership changed. Retry the command.')
            seen[id] = true
        end
        for _, member in ipairs(members) do
            check(member:getRawMetadata('stackInFolderIsCollapsed') == true and
                member:getRawMetadata('stackPositionInFolder') == positions[identifier(member)],
                'Stack expansion or order changed. Retry the command.')
        end
    end
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
    local photos, stacks = collect(selected)
    local changes, positions = {}, nil
    if status == 'reject_hidden' then
        stacks, positions = collapsedStacks(stacks)
        changes = rejectionChanges(stacks, true)
    elseif status == 'sync_rejection' then changes = rejectionChanges(stacks)
    else
        for _, photo in ipairs(photos) do changes[#changes+1] = { photo = photo, status = status } end
    end
    -- Do not create a write/undo operation for an already synchronized selection
    -- or a selection containing only unstacked photos.
    if #changes == 0 then return 0, #stacks end
    local callbackRan = false
    local result = catalog:withWriteAccessDo(action.title, function()
        if status == 'reject_hidden' then verifyCollapsed(stacks, positions) end
        for _, change in ipairs(changes) do change.photo:setRawMetadata('pickStatus', change.status) end
        callbackRan = true
    end, { timeout = 30 })
    check(callbackRan and result == 'executed', 'Catalog busy. Retry the stack flag command.')
    return #changes, #stacks
end
function M.start(status)
    LrTasks.startAsyncTask(function()
        if busy then LrDialogs.showBezel('Stack flags are updating. Try again.', 1.5); return end
        busy = true
        local count, stackCount
        local ok, problem = LrTasks.pcall(function()
            count, stackCount = M.apply(LrApplication.activeCatalog(), status)
        end)
        busy = false
        if ok and status == 'reject_hidden' and count == 0 then
            LrDialogs.showBezel(stackCount == 0 and 'No collapsed folder stacks selected.' or
                'Hidden stack photos are already rejected.', 1.5)
        elseif ok and status == 'sync_rejection' and count == 0 then
            LrDialogs.showBezel(stackCount == 0 and 'No folder stacks selected.' or
                'Rejection already matches stack tops.', 1.5)
        elseif ok then
            LrDialogs.showBezel(actions[status].verb .. ' ' .. tostring(count) ..
                (count == 1 and ' photo' or ' photos'), 1.5)
        else
            LrDialogs.message('Stack flags', tostring(problem), 'critical')
        end
    end)
end
return M
