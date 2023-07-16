create table comments (
    id integer primary key,
    account_id integer not null references accounts(id),
    post_id integer not null references posts(id),
    body text not null,
    updated_at int not null,
    created_at int not null
);