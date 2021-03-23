wrk.method = "POST"
wrk.headers["Content-Type"] = "application/json"

local query = "{ topProducts { upc price reviews { body author { id username } } } }"
local body = string.format("{\"operationName\":null,\"variables\":{},\"query\":\"%s\"}", query)

function request()
    return wrk.format('POST', nil, nil, body)
end