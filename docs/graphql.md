# GraphQL Schema Reference

Maestro exposes a GraphQL API for querying indexed blockchain data. The playground is available at `http://localhost:4000/graphql`.

## Schema Overview

```graphql
type Query {
  # Core queries
  block(number: Int, hash: String): Block
  blocks(first: Int, after: String, order: Order, filter: BlockFilter): BlockConnection!

  extrinsic(id: String!): Extrinsic
  extrinsics(first: Int, after: String, order: Order, filter: ExtrinsicFilter): ExtrinsicConnection!

  event(id: String!): Event
  events(first: Int, after: String, order: Order, filter: EventFilter): EventConnection!

  # Bundle queries (e.g., Balances)
  transfer(id: String!): Transfer
  transfers(first: Int, after: String, order: Order, filter: TransferFilter): TransferConnection!
}
```

## Pagination

All list queries use Relay-style cursor pagination:

```graphql
query {
  blocks(first: 10, after: "cursor123") {
    edges {
      cursor
      node {
        number
        hash
      }
    }
    pageInfo {
      hasNextPage
      hasPreviousPage
      startCursor
      endCursor
    }
  }
}
```

### Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `first` | Int | Number of items to return (max 100) |
| `after` | String | Cursor for forward pagination |
| `last` | Int | Number of items from the end |
| `before` | String | Cursor for backward pagination |
| `order` | Order | Sort order: `ASC` or `DESC` |

## Types

### Block

```graphql
type Block {
  number: Int!
  hash: String!
  parentHash: String!
  stateRoot: String!
  extrinsicsRoot: String!
  author: String
  timestamp: DateTime
  extrinsicCount: Int!
  eventCount: Int!
  indexedAt: DateTime!

  # Relationships
  extrinsics(first: Int, after: String): ExtrinsicConnection!
  events(first: Int, after: String): EventConnection!
}
```

### Extrinsic

```graphql
type Extrinsic {
  id: String!
  blockNumber: Int!
  blockHash: String!
  index: Int!
  pallet: String!
  call: String!
  signer: String
  success: Boolean!
  error: String
  args: JSON!
  raw: String!
  tip: String
  nonce: Int

  # Relationships
  block: Block!
  events: [Event!]!
}
```

### Event

```graphql
type Event {
  id: String!
  blockNumber: Int!
  blockHash: String!
  index: Int!
  extrinsicIndex: Int
  pallet: String!
  name: String!
  data: JSON!
  topics: [String!]!

  # Relationships
  block: Block!
  extrinsic: Extrinsic
}
```

### Transfer (Balances Bundle)

```graphql
type Transfer {
  id: String!
  blockNumber: Int!
  blockHash: String!
  eventIndex: Int!
  extrinsicIndex: Int
  from: String!
  to: String!
  amount: String!
  success: Boolean!
  timestamp: DateTime
}
```

## Filters

### BlockFilter

```graphql
input BlockFilter {
  numberGte: Int      # Block number >= value
  numberLte: Int      # Block number <= value
  author: String      # Block author (validator)
  timestampGte: DateTime
  timestampLte: DateTime
}
```

### ExtrinsicFilter

```graphql
input ExtrinsicFilter {
  blockNumber: Int
  pallet: String      # Filter by pallet name
  call: String        # Filter by call name
  signer: String      # Filter by signer address
  success: Boolean    # Filter by success status
}
```

### EventFilter

```graphql
input EventFilter {
  blockNumber: Int
  pallet: String      # Filter by pallet name
  name: String        # Filter by event name
  extrinsicIndex: Int
}
```

### TransferFilter

```graphql
input TransferFilter {
  blockNumberGte: Int
  blockNumberLte: Int
  from: String        # Sender address
  to: String          # Recipient address
  account: String     # Either sender or recipient
  amountGte: String   # Minimum amount
  amountLte: String   # Maximum amount
}
```

## Example Queries

### Get Latest Blocks

```graphql
query LatestBlocks {
  blocks(first: 10, order: DESC) {
    edges {
      node {
        number
        hash
        timestamp
        extrinsicCount
        eventCount
      }
    }
  }
}
```

### Get Block with Extrinsics and Events

```graphql
query BlockDetails($number: Int!) {
  block(number: $number) {
    number
    hash
    timestamp
    author
    extrinsics(first: 50) {
      edges {
        node {
          index
          pallet
          call
          signer
          success
        }
      }
    }
    events(first: 100) {
      edges {
        node {
          index
          pallet
          name
          data
        }
      }
    }
  }
}
```

### Find Transfers for Account

```graphql
query AccountTransfers($account: String!) {
  transfers(
    first: 20
    order: DESC
    filter: { account: $account }
  ) {
    edges {
      node {
        id
        blockNumber
        from
        to
        amount
        timestamp
      }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
```

### Filter Events by Pallet

```graphql
query StakingEvents {
  events(
    first: 50
    order: DESC
    filter: { pallet: "Staking" }
  ) {
    edges {
      node {
        blockNumber
        name
        data
        extrinsicIndex
      }
    }
  }
}
```

### Get Extrinsic with Events

```graphql
query ExtrinsicDetails($id: String!) {
  extrinsic(id: $id) {
    id
    blockNumber
    pallet
    call
    signer
    success
    error
    args
    events {
      pallet
      name
      data
    }
  }
}
```

## Notes

### Addresses

All addresses are returned as hex strings with `0x` prefix:
```json
"from": "0x1234567890abcdef..."
```

### Amounts

Large numbers (u128) are returned as strings to preserve precision:
```json
"amount": "1000000000000000000"
```

### Timestamps

Timestamps are ISO 8601 format:
```json
"timestamp": "2024-01-15T10:30:00Z"
```

### Rate Limits

The default configuration allows:
- Maximum 100 items per page
- No query depth limit (configure in production)

### Playground

The GraphQL playground is enabled by default and available at `/graphql`. Disable in production if not needed.
