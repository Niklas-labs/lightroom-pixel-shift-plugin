-- Small strict JSON reader for the plugin/engine protocol; Lua 5.1 compatible.
local M = { null = {} }
local function utf8(value)
    if value < 128 then return string.char(value) end
    if value < 2048 then return string.char(192 + math.floor(value / 64), 128 + value % 64) end
    if value < 65536 then return string.char(224 + math.floor(value / 4096), 128 + math.floor(value / 64) % 64, 128 + value % 64) end
    return string.char(240 + math.floor(value / 262144), 128 + math.floor(value / 4096) % 64, 128 + math.floor(value / 64) % 64, 128 + value % 64)
end
function M.decode(source)
    assert(type(source) == 'string', 'Expected JSON text')
    local position, length, depth = 1, #source, 0
    local function fail(message) error('Invalid JSON at byte ' .. position .. ': ' .. message, 0) end
    local function whitespace()
        while source:sub(position, position):match('[ \t\r\n]') and position <= length do position = position + 1 end
    end
    local function hex()
        local value = source:sub(position, position + 3)
        if #value ~= 4 or value:find('[^%da-fA-F]') then fail('invalid Unicode escape') end
        position = position + 4
        return tonumber(value, 16)
    end
    local function stringValue()
        position = position + 1
        local result = {}
        while position <= length do
            local c = source:sub(position, position)
            position = position + 1
            if c == '"' then return table.concat(result) end
            if c == '\\' then
                local escape = source:sub(position, position)
                position = position + 1
                local escapes = { ['"'] = '"', ['\\'] = '\\', ['/'] = '/', b = '\b', f = '\f', n = '\n', r = '\r', t = '\t' }
                if escape == 'u' then
                    local value = hex()
                    if value >= 0xD800 and value <= 0xDBFF then
                        if source:sub(position, position + 1) ~= '\\u' then fail('missing low surrogate') end
                        position = position + 2
                        local low = hex()
                        if low < 0xDC00 or low > 0xDFFF then fail('invalid low surrogate') end
                        value = 0x10000 + (value - 0xD800) * 1024 + low - 0xDC00
                    elseif value >= 0xDC00 and value <= 0xDFFF then fail('unpaired low surrogate') end
                    result[#result + 1] = utf8(value)
                elseif escapes[escape] then result[#result + 1] = escapes[escape]
                else fail('invalid escape') end
            else
                if c:byte() < 32 then fail('unescaped control character') end
                result[#result + 1] = c
            end
        end
        fail('unterminated string')
    end
    local parse
    parse = function()
        whitespace()
        depth = depth + 1
        if depth > 64 then fail('nesting limit exceeded') end
        local c, value = source:sub(position, position)
        if c == '"' then value = stringValue()
        elseif c == '{' or c == '[' then
            local object = c == '{'
            local close = object and '}' or ']'
            position = position + 1
            value = {}
            whitespace()
            if source:sub(position, position) == close then position = position + 1
            else
                while true do
                    whitespace()
                    local key
                    if object then
                        if source:sub(position, position) ~= '"' then fail('expected object key') end
                        key = stringValue()
                        if value[key] ~= nil then fail('duplicate object key') end
                        whitespace()
                        if source:sub(position, position) ~= ':' then fail('expected colon') end
                        position = position + 1
                    else key = #value + 1 end
                    value[key] = parse()
                    whitespace()
                    local delimiter = source:sub(position, position)
                    position = position + 1
                    if delimiter == close then break end
                    if delimiter ~= ',' then fail('expected comma or closing delimiter') end
                end
            end
        elseif source:sub(position, position + 3) == 'true' then position = position + 4; value = true
        elseif source:sub(position, position + 4) == 'false' then position = position + 5; value = false
        elseif source:sub(position, position + 3) == 'null' then position = position + 4; value = M.null
        else
            local tail = source:sub(position)
            local number = tail:match('^-?%d+%.?%d*[eE][+-]?%d+') or tail:match('^-?%d+%.%d+') or tail:match('^-?%d+')
            if not number or number:match('^-?0%d') or number:match('%.e') or number:match('%.E') then fail('expected value') end
            value = tonumber(number)
            if not value or value ~= value or value == math.huge or value == -math.huge then fail('non-finite number') end
            position = position + #number
        end
        depth = depth - 1
        return value
    end
    local value = parse()
    whitespace()
    if position <= length then fail('trailing data') end
    return value
end
return M
