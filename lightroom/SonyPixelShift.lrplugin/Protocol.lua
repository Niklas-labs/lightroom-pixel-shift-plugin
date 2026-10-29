-- Lua 5.1 compatible. Only generated job fields are serialized.
local M = {}
function M.jsonString(value)
    return '"' .. value:gsub('[%z\1-\31\\"]', function(c)
        if c == '\\' then return '\\\\' end
        if c == '"' then return '\\"' end
        return string.format('\\u%04x', string.byte(c))
    end) .. '"'
end
function M.job(paths, output, mode)
    local inputs = {}
    for i, path in ipairs(paths) do inputs[i] = M.jsonString(path) end
    return '{"version":1,"camera":"ILCE-7CR","mode":' .. M.jsonString(mode)
        .. ',"inputs":[' .. table.concat(inputs, ',') .. '],"output":'
        .. M.jsonString(output) .. ',"motion_threshold":0.08,"alignment_radius":2}'
end
function M.quote(path, windows)
    assert(not path:find('[%z\r\n]'), 'Invalid command path')
    if windows then
        -- cmd.exe expansion is not made safe by double quotes. Reject expansion
        -- and control characters; paths with spaces and Unicode remain allowed.
        assert(not path:find('[%%!%^&|<>"()]'), 'Unsupported Windows command path character')
        return '"' .. path .. '"'
    end
    return "'" .. path:gsub("'", "'\\''") .. "'"
end
return M
